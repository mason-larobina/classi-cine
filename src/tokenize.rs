use crate::tokens::*;
use log::*;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::io::Write;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use crate::bloom::Bloom;

#[derive(Debug)]
struct PairCounts {
    counts: Vec<HashMap<Pair, i64>>,
}

impl PairCounts {
    fn new() -> Self {
        let mut counts = Vec::with_capacity(64);
        for _ in 0..64 {
            counts.push(HashMap::new());
        }
        Self { counts }
    }

    fn get_map<'a, T: Hash + Copy>(&'a mut self, item: T) -> &'a mut HashMap<Pair, i64> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        item.hash(&mut hasher);
        let len = self.counts.len() as u64;
        let index = hasher.finish() % len;
        &mut self.counts[index as usize]
    }

    fn update(&mut self, tokens: &Tokens, delta: i64, skip: &[Token]) {
        if delta == 0 {
            return;
        }
        for pair in tokens.pairs() {
            if skip.contains(&pair.0) || skip.contains(&pair.1) {
                continue;
            }

            match self.get_map(pair).entry(pair) {
                Entry::Occupied(mut entry) => {
                    let count = entry.get().saturating_add(delta);
                    if count == 0 {
                        entry.remove();
                    } else {
                        entry.insert(count);
                    }
                }
                Entry::Vacant(entry) => {
                    entry.insert(delta);
                }
            }
        }
    }

    fn map_max_count(counts: &HashMap<Pair, i64>) -> Option<(i64, Pair)> {
        counts.iter().map(|(pair, count)| (*count, *pair)).max()
    }

    fn max(&self) -> Option<(i64, Pair)> {
        self.counts.par_iter()
            .filter_map(Self::map_max_count)
            .max()
    }
}

#[derive(Debug)]
pub struct PairTokenizer {
    pub(crate) token_map: TokenMap,
    skip: Vec<Token>,
    merges: Vec<(Pair, Token)>,
}

impl PairTokenizer {
    pub fn new(strings: Vec<String>) -> PairTokenizer {
        assert!(strings.len() > 0);

        let mut token_map = TokenMap::new();
        let skip = vec![
            token_map.create_token(std::path::MAIN_SEPARATOR_STR),
            token_map.create_token(" "),
        ];

        let min_freq: i64 = (strings.len() as f64).log2() as i64;
        let mut strings: Vec<Tokens> = strings.into_iter().map(|s| Tokens::from_str_and_create(&s, &mut token_map)).collect();

        let mut merges = Vec::new();
        let mut counts = PairCounts::new();
        let mut tmp = Tokens::default();

        for s in strings.iter() {
            counts.update(s, 1, &skip);
        }

        loop {
            // Find max pair or finish.
            let pair = if let Some((count, pair)) = counts.max() {
                if count < min_freq {
                    break;
                }
                info!("{:?} {:?}", count, pair.to_string(&token_map));
                pair
            } else {
                break;
            };

            // Merge pair into new token.
            let merged = token_map.merge(pair);
            merges.push((pair, merged));

            let mut bloom = Bloom::default();
            bloom.set(pair);

            // Apply merges and update counts.
            for s in strings.iter_mut() {
                if s.contains(&bloom) {
                    if tmp.from_replace(s, pair, merged, &skip) {
                        counts.update(s, -1, &skip);
                        counts.update(&tmp, 1, &skip);
                        std::mem::swap(&mut tmp, s);
                    }
                }
            }
        }

        PairTokenizer {
            token_map,
            skip,
            merges,
        }
    }

    pub fn tokenize(&self, s: &str) -> Tokens {
        let mut tokens = Tokens::from_str_or_unknown(&s, &self.token_map);
        let mut tmp = Tokens::default();
        for (pair, merged) in self.merges.iter().cloned() {
            if tokens.contains(&pair) {
                if tmp.from_replace(&tokens, pair, merged, &self.skip) {
                    std::mem::swap(&mut tmp, &mut tokens);
                }
            }
        }
        tokens
   }
}
