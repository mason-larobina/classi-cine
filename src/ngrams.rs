use crate::tokenize::PairTokenizer;
use crate::tokens::{Token, Tokens};
use ahash::{AHashMap, AHashSet};
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Copy, Clone)]
pub struct Ngram(pub(crate) u64);

#[derive(Debug, Default, Clone)]
pub struct Ngrams(Vec<Ngram>);

impl Ngram {
    pub fn new(tokens: &[Token]) -> Self {
        let mut hasher = DefaultHasher::new();
        tokens.hash(&mut hasher);
        Self(hasher.finish())
    }
}

impl Ngrams {
    pub fn windows(
        &mut self,
        tokens: &Tokens,
        windows: usize,
        allowed: Option<&AHashSet<Ngram>>,
        mut debug: Option<&mut Vec<Vec<Token>>>,
    ) {
        self.0.clear();
        for n in 1..=windows {
            for window in tokens.as_slice().windows(n) {
                let ngram = Ngram::new(window);
                if let Some(allowed) = &allowed
                    && !allowed.contains(&ngram)
                {
                    continue;
                }
                self.0.push(ngram);
                if let Some(d) = &mut debug {
                    d.push(window.to_vec());
                }
            }
        }
        self.0.sort();
        self.0.dedup();
    }

    /// Generates *orderless* combinations of non-special tokens and appends
    /// them to this `Ngrams` (augmenting whatever contiguous `windows` already
    /// produced). Each combination is sorted before hashing so `{a, b}` and
    /// `{b, a}` collapse to the same [`Ngram`]. Special tokens (those with id
    /// `<= last_special`, e.g. `<ROOT>`, `<EOL>`, `<UNK>`) are excluded since
    /// they are structural and would dominate the feature set.
    ///
    /// Sizes range over `2..=k`: size-1 combinations are identical to the
    /// size-1 windows already produced by [`Ngrams::windows`], so they are
    /// skipped to avoid redundant work.
    pub fn extend_combinations(
        &mut self,
        tokens: &Tokens,
        k: usize,
        last_special: Token,
        allowed: Option<&AHashSet<Ngram>>,
        mut debug: Option<&mut Vec<Vec<Token>>>,
    ) {
        // Restrict to non-special tokens: combination features should describe
        // actual content, not structural sentinels shared by every path.
        let filtered: Vec<Token> = tokens
            .as_slice()
            .iter()
            .copied()
            .filter(|t| *t > last_special)
            .collect();
        let len = filtered.len();
        for n in 2..=k {
            if n > len {
                break;
            }
            for combo in CombinationIter::new(len, n) {
                let mut window: Vec<Token> = combo.into_iter().map(|i| filtered[i]).collect();
                window.sort();
                let ngram = Ngram::new(&window);
                if let Some(allowed) = &allowed
                    && !allowed.contains(&ngram)
                {
                    continue;
                }
                self.0.push(ngram);
                if let Some(d) = &mut debug {
                    d.push(window);
                }
            }
        }
        self.0.sort();
        self.0.dedup();
    }

    pub fn iter(&self) -> impl Iterator<Item = &Ngram> {
        self.0.iter()
    }

    /// Counts ngrams from a collection of paths, filters for frequent ones, and returns the set.
    ///
    /// This function tokenizes each path using the provided tokenizer, generates ngrams
    /// for each token sequence, counts their occurrences across all paths, and returns
    /// a set of ngrams that appear more than once.
    pub fn count_and_filter_from_paths(
        paths: &[String],
        tokenizer: &PairTokenizer,
        windows: usize,
        combinations: usize,
    ) -> AHashSet<Ngram> {
        let chunk_size = usize::max(100, paths.len() / (num_cpus::get() * 10));
        let last_special = tokenizer.token_map().last_special();

        let ngram_counts: AHashMap<Ngram, u8> = paths
            .par_chunks(chunk_size)
            .map(|chunk| {
                let mut local_counts: AHashMap<Ngram, u8> = AHashMap::new();
                let mut temp_ngrams = Ngrams::default(); // Use Ngrams struct
                for path in chunk {
                    let tokens = tokenizer.tokenize(path); // Tokenize each path
                    // Generate ngrams for counting without filtering
                    temp_ngrams.windows(&tokens, windows, None, None);
                    temp_ngrams.extend_combinations(
                        &tokens,
                        combinations,
                        last_special,
                        None,
                        None,
                    );
                    for ngram in temp_ngrams.iter() {
                        let counter = local_counts.entry(*ngram).or_default();
                        *counter = counter.saturating_add(1);
                    }
                }
                local_counts
            })
            .reduce(AHashMap::new, |mut acc, local_counts| {
                // Reduction: merge local counts into accumulator
                for (ngram, count) in local_counts {
                    let counter = acc.entry(ngram).or_insert(0);
                    *counter = counter.saturating_add(count);
                }
                acc
            });

        // Filter to frequent ngrams (count > 1)
        ngram_counts
            .into_iter()
            .filter_map(|(ngram, count)| if count > 1 { Some(ngram) } else { None })
            .collect()
    }
}

/// Lexicographic iterator over the `n`-combinations of indices `0..len`.
///
/// Yields each combination as an ascending sequence of indices (e.g. for
/// `len=4, n=2`: `[0,1], [0,2], [0,3], [1,2], [1,3], [2,3]`). Yields nothing
/// when `n > len` or `n == 0`.
struct CombinationIter {
    indices: Vec<usize>,
    len: usize,
    n: usize,
    done: bool,
    first: bool,
}

impl CombinationIter {
    fn new(len: usize, n: usize) -> Self {
        Self {
            indices: (0..n).collect(),
            len,
            n,
            done: n == 0 || n > len,
            first: true,
        }
    }
}

impl Iterator for CombinationIter {
    type Item = Vec<usize>;

    fn next(&mut self) -> Option<Vec<usize>> {
        if self.done {
            return None;
        }
        if self.first {
            self.first = false;
            return Some(self.indices.clone());
        }
        let n = self.n;
        let len = self.len;
        // Find the rightmost index that can still be incremented.
        let mut i: isize = n as isize - 1;
        while i >= 0 && self.indices[i as usize] == i as usize + (len - n) {
            i -= 1;
        }
        if i < 0 {
            self.done = true;
            return None;
        }
        self.indices[i as usize] += 1;
        for j in (i as usize + 1)..n {
            self.indices[j] = self.indices[j - 1] + 1;
        }
        Some(self.indices.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::TokenMap;

    fn tokens_from(s: &str, map: &mut TokenMap) -> Tokens {
        Tokens::from_str_and_create(s, map)
    }

    #[test]
    fn combination_iter_counts() {
        // C(4, 2) = 6
        let combos: Vec<Vec<usize>> = CombinationIter::new(4, 2).collect();
        assert_eq!(combos.len(), 6);
        assert_eq!(combos[0], vec![0, 1]);
        assert_eq!(combos[5], vec![2, 3]);

        // C(5, 3) = 10
        assert_eq!(CombinationIter::new(5, 3).count(), 10);

        // n > len yields nothing; n == 0 yields nothing.
        assert_eq!(CombinationIter::new(3, 4).count(), 0);
        assert_eq!(CombinationIter::new(3, 0).count(), 0);

        // C(6, 2) = 15
        assert_eq!(CombinationIter::new(6, 2).count(), 15);
    }

    #[test]
    fn combinations_are_orderless() {
        let mut map = TokenMap::new("");
        let last_special = map.last_special();
        // "ab" and "ba" share the same character tokens; their pair
        // combination must hash to a single ngram regardless of order.
        let t_ab = tokens_from("ab", &mut map);
        let t_ba = tokens_from("ba", &mut map);

        let mut a = Ngrams::default();
        a.extend_combinations(&t_ab, 2, last_special, None, None);
        let mut b = Ngrams::default();
        b.extend_combinations(&t_ba, 2, last_special, None, None);

        // Both produce the same multiset of ngrams.
        let mut av: Vec<_> = a.iter().copied().collect();
        let mut bv: Vec<_> = b.iter().copied().collect();
        av.sort();
        bv.sort();
        assert_eq!(av, bv);
        // Just the single pair {a, b} after specials are filtered out.
        assert_eq!(av.len(), 1);
    }

    #[test]
    fn combinations_skip_specials_and_start_at_size_2() {
        let mut map = TokenMap::new("");
        let last_special = map.last_special();
        // tokens for "abc": <root>, a, b, c, <EOL>. Only a, b, c are
        // non-special, so the combination pool has 3 elements.
        let t = tokens_from("abc", &mut map);

        let mut n = Ngrams::default();
        n.extend_combinations(&t, 2, last_special, None, None);
        // Only pairs: C(3, 2) = 3 (size-1 is deliberately skipped).
        assert_eq!(n.iter().count(), 3);

        let mut n = Ngrams::default();
        n.extend_combinations(&t, 3, last_special, None, None);
        // Pairs + triples: C(3,2) + C(3,3) = 3 + 1 = 4.
        assert_eq!(n.iter().count(), 4);

        // A path with only one non-special token yields no combinations
        // since the smallest size is 2.
        let t1 = tokens_from("a", &mut map);
        let mut n = Ngrams::default();
        n.extend_combinations(&t1, 3, last_special, None, None);
        assert_eq!(n.iter().count(), 0);
    }
}
