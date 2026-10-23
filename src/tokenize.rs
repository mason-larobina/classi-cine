use crate::bloom::{Bloom, BloomMask};
use crate::tokens::*;
use ahash::AHashMap as HashMap;
use log::*;
use rayon::prelude::*;
use std::collections::BinaryHeap;
use std::sync::Mutex;

/// A tokenization structure that merges frequently co-occurring token pairs
/// and stores those merges for subsequent use.
#[derive(Debug)]
pub struct PairTokenizer {
    token_map: TokenMap,
    /// Each merge with its prehashed Bloom mask, so `tokenize` doesn't rehash
    /// every pair for every path.
    merges: Vec<(Pair, Token, BloomMask)>,
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
            .map(|s| Tokens::from_str_and_create(s, &mut token_map))
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
        let mut merges: Vec<(Pair, Token)> = Vec::new();

        // Populate initial pair counts by iterating over all strings and their pairs.
        let mut counts: HashMap<Pair, i64> = HashMap::new();
        for s in strings.iter() {
            for p in s.pairs(&token_map) {
                *counts.entry(p).or_insert(0) += 1;
            }
        }

        // Lazy max-heap of `(count, pair)`. Entries whose count no longer matches
        // `counts` are stale and are discarded when popped.
        let mut heap: BinaryHeap<(i64, Pair)> = counts.iter().map(|(p, c)| (*c, *p)).collect();

        // Determine the chunk size for parallel processing of strings.
        let chunk_size = usize::max(100, strings.len() / num_cpus::get());

        // Continuously find and merge the most frequent pair until no pairs exceed the min_freq threshold.
        loop {
            // Pop until an up-to-date entry is found; it is the max by `(count, pair)`.
            let pair = loop {
                match heap.pop() {
                    Some((count, pair)) => {
                        if counts.get(&pair) != Some(&count) {
                            continue;
                        }
                        // If the highest frequency is below min_freq, we are done.
                        if count < min_freq {
                            break None;
                        }
                        break Some(pair);
                    }
                    None => break None,
                }
            };
            let Some(pair) = pair else {
                break;
            };

            // Merge the chosen pair into a single new token in the token map.
            let merged = token_map.merge(pair);
            merges.push((pair, merged));

            // Prepare a Bloom mask to quickly check if a Tokens object might contain this pair.
            let mask = Bloom::mask(pair);

            // References for use in parallel processing.
            let token_map_ref = &token_map;
            let last_special = token_map.last_special();
            let deltas: Mutex<Vec<HashMap<Pair, i64>>> = Mutex::new(Vec::new());
            let deltas_ref = &deltas;

            // Process the strings in chunks in parallel.
            strings.par_chunks_mut(chunk_size).for_each(move |chunk| {
                // `delta` accumulates changes to pair counts that happen within this chunk.
                let mut delta: HashMap<Pair, i64> = HashMap::new();
                // `tmp` is a temporary Tokens structure used during replacement.
                let mut tmp = Tokens::default();
                // Which positions of the old string were part of a merge site.
                let mut consumed: Vec<bool> = Vec::new();

                // Iterate over each string in the chunk.
                for s in chunk {
                    // If the string can't possibly have the pair (not in Bloom filter), skip it.
                    if !s.contains(mask) {
                        continue;
                    }
                    // Attempt to replace all occurrences of `pair` with the merged token in `s`.
                    if tmp.replace_with_consumed(token_map_ref, s, pair, merged, &mut consumed) {
                        // Only windows touching a merge site change: remove the old ones...
                        for (i, w) in s.as_slice().windows(2).enumerate() {
                            if (consumed[i] || consumed[i + 1])
                                && w[0] > last_special
                                && w[1] > last_special
                            {
                                *delta.entry(Pair(w[0], w[1])).or_insert(0) -= 1;
                            }
                        }
                        // ...and add the new ones involving the merged token.
                        for w in tmp.as_slice().windows(2) {
                            if (w[0] == merged || w[1] == merged)
                                && w[0] > last_special
                                && w[1] > last_special
                            {
                                *delta.entry(Pair(w[0], w[1])).or_insert(0) += 1;
                            }
                        }
                        // Swap `s` with `tmp` to finalize the updated tokens.
                        std::mem::swap(s, &mut tmp);
                    }
                }
                deltas_ref.lock().unwrap().push(delta);
            });

            // Fold the chunk deltas into one total, then apply it to counts and the heap.
            let mut total: HashMap<Pair, i64> = HashMap::new();
            for delta in deltas.into_inner().unwrap() {
                for (p, v) in delta {
                    *total.entry(p).or_insert(0) += v;
                }
            }
            for (p, v) in total {
                if v == 0 {
                    continue;
                }
                let new_count = counts.get(&p).copied().unwrap_or(0) + v;
                if new_count == 0 {
                    counts.remove(&p);
                } else {
                    counts.insert(p, new_count);
                    heap.push((new_count, p));
                }
            }
        }

        // Precompute Bloom masks so `tokenize` doesn't rehash each pair per path.
        let merges = merges
            .into_iter()
            .map(|(pair, merged)| (pair, merged, Bloom::mask(pair)))
            .collect();

        // Return a `PairTokenizer` that includes the final token map and merges performed.
        PairTokenizer { token_map, merges }
    }

    /// Tokenizes a given input string `s` by using the merges derived during `new`.
    /// As merges were learned from a corpus, this attempts to reapply those merges in sequence.
    /// For best results, use the same corpus or one of similar distribution.
    pub fn tokenize(&self, s: &str) -> Tokens {
        // Create tokens, using "unknown" tokens if not present in the token_map training corpus.
        let mut tokens = Tokens::from_str_or_unknown(s, &self.token_map);
        // A temporary structure to apply merges one by one.
        let mut tmp = Tokens::default();
        // For each learned merge, apply it if `tokens` contains the pair.
        for (pair, merged, mask) in self.merges.iter().copied() {
            if tokens.contains(mask) && tmp.replace_with(&self.token_map, &tokens, pair, merged) {
                // Swap to finalize the replacement result.
                std::mem::swap(&mut tmp, &mut tokens);
            }
        }
        tokens
    }

    pub fn token_map(&self) -> &TokenMap {
        &self.token_map
    }

    /// Mutable access to the shared token map. Used by the feature pipeline
    /// (`features.rs`) to mint feature-token ids into the **same** map the
    /// path tokenizer owns, so feature ngram hashes coexist safely with path
    /// ngram hashes in a merged `Ngrams` vec. Feature strings are never fed to
    /// `tokenize`; the map is only used for `get_or_create_token`.
    pub fn token_map_mut(&mut self) -> &mut TokenMap {
        &mut self.token_map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Naive reference trainer: recounts every pair from scratch each round.
    fn reference_train(corpus: &[String]) -> (Vec<(Pair, Token)>, Vec<Tokens>) {
        let mut special_chars = String::new();
        special_chars.push(' ');
        special_chars.push(std::path::MAIN_SEPARATOR);
        let mut token_map = TokenMap::new(&special_chars);
        let mut strings: Vec<Tokens> = corpus
            .iter()
            .map(|s| Tokens::from_str_and_create(s, &mut token_map))
            .collect();
        let min_freq: i64 = i64::max(2, (strings.len() as f64 + 1.0).log2() as i64);
        let mut merges = Vec::new();
        loop {
            let mut counts: std::collections::HashMap<Pair, i64> = Default::default();
            for s in &strings {
                for p in s.pairs(&token_map) {
                    *counts.entry(p).or_insert(0) += 1;
                }
            }
            let Some((count, pair)) = counts.iter().map(|(p, c)| (*c, *p)).max() else {
                break;
            };
            if count < min_freq {
                break;
            }
            let merged = token_map.merge(pair);
            merges.push((pair, merged));
            let mut tmp = Tokens::default();
            for s in strings.iter_mut() {
                if tmp.replace_with(&token_map, s, pair, merged) {
                    std::mem::swap(s, &mut tmp);
                }
            }
        }
        (merges, strings)
    }

    fn synthetic_corpus() -> Vec<String> {
        let sep = std::path::MAIN_SEPARATOR;
        let dirs = ["media", "movies", "tv shows", "home videos", "archive"];
        let words = [
            "holiday", "summer", "episode", "season", "final", "cut", "trailer", "beach", "family",
            "birthday",
        ];
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut next = move |n: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        let mut out = Vec::new();
        for i in 0..2000 {
            let d1 = dirs[next(dirs.len())];
            let d2 = words[next(words.len())];
            let w1 = words[next(words.len())];
            let w2 = words[next(words.len())];
            let name = match i % 5 {
                0 => format!("{w1} {w2} {}", next(100)),
                1 => format!("{w1}_{w2}_s{:02}e{:02}", next(10), next(30)),
                2 => format!("aaaa{w1}abab{}ababab", "aaaaaa"),
                3 => format!("{}{w1}", "abab".repeat(1 + next(5))),
                _ => format!("{w1}-{w2}-{w1}-{w2}-{}", "a".repeat(2 + next(8))),
            };
            out.push(format!("{d1}{sep}{d2}{sep}{name}.mkv"));
        }
        out
    }

    #[test]
    fn test_matches_naive_reference() {
        let corpus = synthetic_corpus();
        let (ref_merges, ref_strings) = reference_train(&corpus);
        assert!(ref_merges.len() > 20, "corpus should yield many merges");

        let pt = PairTokenizer::new(corpus.iter().map(String::as_str));
        let got: Vec<(Pair, Token)> = pt.merges.iter().map(|(p, t, _)| (*p, *t)).collect();
        assert_eq!(got, ref_merges);

        for (s, expected) in corpus.iter().zip(ref_strings.iter()) {
            assert_eq!(pt.tokenize(s).as_slice(), expected.as_slice(), "{s}");
        }
    }

    #[test]
    fn test_pair_tokenizer_basic() {
        // Use repeated input to exceed the minimum frequency threshold
        let training = ["hello world"; 10];
        let pt = PairTokenizer::new(training.iter().copied());
        let tokens = pt.tokenize("hello world");
        // Checking if the round trip matches
        assert_eq!(
            tokens.debug_strs(&pt.token_map),
            vec!["<ROOT>", "hello", " ", "world", "<EOL>"]
        );
    }
}
