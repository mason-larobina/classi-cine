use crate::tokenize::PairTokenizer;
use crate::tokens::{Token, Tokens};
use ahash::{AHashMap, AHashSet};
use num_cpus;
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Copy, Clone)]
pub struct Ngram(pub(crate) u64);

#[derive(Debug, Default)]
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
                if let Some(allowed) = &allowed {
                    if !allowed.contains(&ngram) {
                        continue;
                    }
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

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = &'a Ngram> {
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
    ) -> AHashSet<Ngram> {
        let chunk_size = usize::max(100, paths.len() / (num_cpus::get() * 10));

        let ngram_counts: AHashMap<Ngram, u8> = paths
            .par_chunks(chunk_size)
            .map(|chunk| {
                let mut local_counts: AHashMap<Ngram, u8> = AHashMap::new();
                let mut temp_ngrams = Ngrams::default(); // Use Ngrams struct
                for path in chunk {
                    let tokens = tokenizer.tokenize(path); // Tokenize each path
                    // Generate ngrams for counting without filtering
                    temp_ngrams.windows(&tokens, windows, None, None);
                    for ngram in temp_ngrams.iter() {
                        let counter = local_counts.entry(*ngram).or_default();
                        *counter = counter.saturating_add(1);
                    }
                }
                local_counts
            })
            .reduce(
                || AHashMap::new(),
                |mut acc, local_counts| {
                    // Reduction: merge local counts into accumulator
                    for (ngram, count) in local_counts {
                        let counter = acc.entry(ngram).or_insert(0);
                        *counter = counter.saturating_add(count);
                    }
                    acc
                },
            );

        // Filter to frequent ngrams (count > 1)
        ngram_counts
            .into_iter()
            .filter_map(|(ngram, count)| if count > 1 { Some(ngram) } else { None })
            .collect()
    }
}
