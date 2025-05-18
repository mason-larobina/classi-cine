use crate::bloom::Bloom;
use crate::tokens::*;
use ahash::{AHashMap as HashMap, AHasher};
use log::*;
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

trait PairUpdater {
    /// Modifies the count of `pair` by an amount `delta`.
    /// If the resulting count is zero, it removes the pair from the data structure.
    fn update_pair(&mut self, pair: Pair, delta: i64);
}

impl PairUpdater for HashMap<Pair, i64> {
    fn update_pair(&mut self, pair: Pair, delta: i64) {
        match self.entry(pair) {
            Entry::Occupied(mut entry) => {
                // Update the existing count by adding delta.
                let count = entry.get().saturating_add(delta);
                // If the count goes to zero, remove the entry entirely.
                if count == 0 {
                    entry.remove();
                } else {
                    entry.insert(count);
                }
            }
            // If the pair is not present, insert it with the new delta as the count.
            Entry::Vacant(entry) => {
                entry.insert(delta);
            }
        }
    }
}

impl PairUpdater for PairCounts {
    fn update_pair(&mut self, pair: Pair, delta: i64) {
        let map = self.get_map(pair);
        map.update_pair(pair, delta);
    }
}

/// A structure that maintains pair counts across multiple shards (HashMaps).
/// Each shard stores a portion of key/value pairs based on hashing.
#[derive(Debug)]
struct PairCounts {
    counts: Vec<HashMap<Pair, i64>>,
}

impl PairCounts {
    fn new() -> Self {
        // Determine how many shards to create based on the number of CPU cores.
        let shards = num_cpus::get();
        let mut counts = Vec::with_capacity(shards);
        for _ in 0..shards {
            counts.push(HashMap::new());
        }
        Self { counts }
    }

    /// Returns a mutable reference to the correct shard for `item`,
    /// determined by hashing `item` modulo the number of shards.
    fn get_map<'a, T: Hash + Copy>(&'a mut self, item: T) -> &'a mut HashMap<Pair, i64> {
        let mut hasher = AHasher::default();
        item.hash(&mut hasher);
        // Modulo the result by the number of shards to pick the correct index.
        let len = self.counts.len() as u64;
        let index = hasher.finish() % len;
        &mut self.counts[index as usize]
    }

    /// Finds the pair with the maximum count within a single shard's HashMap.
    /// Returns `(count, pair)` if found, or None if the shard is empty.
    fn map_max_count(counts: &HashMap<Pair, i64>) -> Option<(i64, Pair)> {
        counts.iter().map(|(pair, count)| (*count, *pair)).max()
    }

    /// Finds the pair with the maximum count by looking across all shards in parallel.
    /// Returns `(count, pair)` if found, or None if empty.
    fn max(&self) -> Option<(i64, Pair)> {
        self.counts.par_iter().filter_map(Self::map_max_count).max()
    }

    /// Applies a batch of updates (stored in `delta`) to the pair counts.
    fn apply(&mut self, delta: HashMap<Pair, i64>) {
        for (pair, delta) in delta {
            let map = self.get_map(pair);
            map.update_pair(pair, delta);
        }
    }
}

/// A tokenization structure that merges frequently co-occurring token pairs
/// and stores those merges for subsequent use.
#[derive(Debug)]
pub struct PairTokenizer {
    token_map: TokenMap,
    merges: Vec<(Pair, Token)>,
}

impl PairTokenizer {
    /// Constructs a new `PairTokenizer` from a list of input strings.
    /// Internally, it iteratively merges the most frequent pair until a frequency threshold is reached.
    /// Creates a new PairTokenizer from a set of training strings
    ///
    /// # Arguments
    /// * `strings` - Training corpus to learn token pairs from
    ///
    /// # Returns
    /// A new PairTokenizer configured based on the training data
    ///
    /// # Panics
    /// Will panic if strings is empty
    ///
    pub fn new<'a, I>(strings: I) -> PairTokenizer
    where
        I: IntoIterator<Item = &'a str>,
    {
        // Prevent the tokenizer from merging special characters.
        let mut special_chars = String::new();
        special_chars.push(' ');
        special_chars.push(std::path::MAIN_SEPARATOR);

        let mut token_map = TokenMap::new(&special_chars);

        // Transform each string into a sequence of tokens, creating any necessary tokens in the token_map.
        let mut strings: Vec<Tokens> = strings
            .into_iter()
            .map(|s| Tokens::from_str_and_create(&s, &mut token_map))
            .collect();

        if strings.is_empty() {
            return PairTokenizer {
                token_map,
                merges: Vec::new(),
            };
        }

        // Compute the minimum pair merge frequency threshold, derived from the log base 2 of input size.
        // This is a crude form of stemming that prevents merging very rare pairs.
        // For example: cook-ing, cook-er, cook-ed, etc retain the stem/root unless very common.
        let min_freq: i64 = i64::max(2, (strings.len() as f64 + 1.0).log2() as i64);
        info!("merge min freq {:?}", min_freq);

        // Holds the record of merges performed as `(Pair, Token)`.
        let mut merges = Vec::new();

        // Shared, sharded structure to count how often each pair appears.
        let counts = Mutex::new(PairCounts::new());
        // A reference to use inside parallel closures.
        let counts_ref = &counts;

        // Populate initial pair counts by iterating over all strings and their pairs.
        {
            let mut pair_counts_h = counts.lock().unwrap();
            for s in strings.iter() {
                for p in s.pairs(&token_map) {
                    pair_counts_h.update_pair(p, 1);
                }
            }
        }

        // Determine the chunk size for parallel processing of strings.
        let chunk_size = usize::max(100, strings.len() / num_cpus::get());

        // Continuously find and merge the most frequent pair until no pairs exceed the min_freq threshold.
        loop {
            // Find the pair with the maximum count across all shards.
            let pair = match counts.lock().unwrap().max() {
                Some((count, pair)) => {
                    // If the highest frequency is below min_freq, break out of the loop.
                    if count < min_freq {
                        break;
                    }
                    // Log and keep track of the pair chosen for merging.
                    //info!("{:?} {:?}", count, pair.to_string(&token_map));
                    pair
                }
                _ => {
                    // If no pairs are found, break.
                    break;
                }
            };

            // Merge the chosen pair into a single new token in the token map.
            let merged = token_map.merge(pair);
            merges.push((pair, merged));

            // Prepare a Bloom filter to quickly check if a Tokens object might contain this pair.
            let mut bloom = Bloom::default();
            bloom.set(pair);

            // Reference to the token_map for use in parallel processing.
            let token_map_ref = &token_map;

            // Process the strings in chunks in parallel.
            strings.par_chunks_mut(chunk_size).for_each(move |chunk| {
                // `delta` accumulates changes to pair counts that happen within this chunk.
                let mut delta = HashMap::new();
                // `tmp` is a temporary Tokens structure used during replacement.
                let mut tmp = Tokens::default();

                // Iterate over each string in the chunk.
                for s in chunk {
                    // If the string can't possibly have the pair (not in Bloom filter), skip it.
                    if !s.contains(&bloom) {
                        continue;
                    }
                    // Attempt to replace all occurrences of `pair` with the merged token in `s`.
                    if tmp.from_replace(token_map_ref, s, pair, merged) {
                        // If replacements occurred, we adjust the pair counts accordingly.
                        for p in s.pairs(token_map_ref) {
                            delta.update_pair(p, -1);
                        }
                        for p in tmp.pairs(token_map_ref) {
                            delta.update_pair(p, 1);
                        }
                        // Swap `s` with `tmp` to finalize the updated tokens.
                        std::mem::swap(s, &mut tmp);
                    }
                }
                // Apply the collected updates from this chunk to the global pair counts.
                counts_ref.lock().unwrap().apply(delta);
            });
        }

        // Return a `PairTokenizer` that includes the final token map and merges performed.
        PairTokenizer { token_map, merges }
    }

    /// Tokenizes a given input string `s` by using the merges derived during `new`.
    /// As merges were learned from a corpus, this attempts to reapply those merges in sequence.
    /// For best results, use the same corpus or one of similar distribution.
    pub fn tokenize(&self, s: &str) -> Tokens {
        // Create tokens, using "unknown" tokens if not present in the token_map training corpus.
        let mut tokens = Tokens::from_str_or_unknown(&s, &self.token_map);
        // A temporary structure to apply merges one by one.
        let mut tmp = Tokens::default();
        // For each learned merge, apply it if `tokens` contains the pair.
        for (pair, merged) in self.merges.iter().cloned() {
            if tokens.contains(&pair) {
                if tmp.from_replace(&self.token_map, &tokens, pair, merged) {
                    // Swap to finalize the replacement result.
                    std::mem::swap(&mut tmp, &mut tokens);
                }
            }
        }
        tokens
    }

    pub fn token_map(&self) -> &TokenMap {
        &self.token_map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_map_pair_updater() {
        let mut map = HashMap::new();
        let pair = Pair(Token(0), Token(1));
        map.update_pair(pair, 3);
        assert_eq!(map.get(&pair), Some(&3));
        map.update_pair(pair, -3);
        assert!(!map.contains_key(&pair));
    }

    #[test]
    fn test_pair_counts_update_and_max() {
        let mut pc = PairCounts::new();
        let pair = Pair(Token(1), Token(2));
        pc.update_pair(pair, 5);
        let max_pair = pc.max().unwrap();
        assert_eq!(max_pair.0, 5);
        assert_eq!(max_pair.1, pair);
        pc.update_pair(pair, -5);
        assert!(pc.max().is_none());
    }

    #[test]
    fn test_pair_tokenizer_basic() {
        // Use repeated input to exceed the minimum frequency threshold
        let training = vec!["hello world"; 10];
        let pt = PairTokenizer::new(training.iter().map(|s| *s));
        let tokens = pt.tokenize("hello world");
        // Checking if the round trip matches
        assert_eq!(
            tokens.debug_strs(&pt.token_map),
            vec!["hello", " ", "world"]
        );
    }
}
