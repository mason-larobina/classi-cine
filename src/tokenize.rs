use crate::bloom::Bloom;
use crate::tokens::*;
use log::*;
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
        let mut hasher = ahash::AHasher::default();
        item.hash(&mut hasher);
        let len = self.counts.len() as u64;
        let index = hasher.finish() % len;
        &mut self.counts[index as usize]
    }

    fn update_pair(map: &mut HashMap<Pair, i64>, pair: Pair, delta: i64) {
        match map.entry(pair) {
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

    fn update(&mut self, token_map: &TokenMap, tokens: &Tokens, delta: i64) {
        if delta == 0 {
            return;
        }
        for pair in tokens.pairs(token_map) {
            let map = self.get_map(pair);
            Self::update_pair(map, pair, delta);
        }
    }

    fn map_max_count(counts: &HashMap<Pair, i64>) -> Option<(i64, Pair)> {
        counts.iter().map(|(pair, count)| (*count, *pair)).max()
    }

    fn max(&self) -> Option<(i64, Pair)> {
        self.counts.par_iter().filter_map(Self::map_max_count).max()
    }

    fn apply(&mut self, delta: HashMap<Pair, i64>) {
        for (pair, delta) in delta {
            let map = self.get_map(pair);
            Self::update_pair(map, pair, delta);
        }
    }
}

#[derive(Debug)]
pub struct PairTokenizer {
    pub(crate) token_map: TokenMap,
    merges: Vec<(Pair, Token)>,
}

impl PairTokenizer {
    pub fn new(strings: Vec<String>) -> PairTokenizer {
        assert!(strings.len() > 0);

        let special_chars = format!(" {}", std::path::MAIN_SEPARATOR);
        let mut token_map = TokenMap::new(&special_chars);

        let min_freq: i64 = (strings.len() as f64).log2() as i64;
        let mut strings: Vec<Tokens> = strings
            .into_iter()
            .map(|s| Tokens::from_str_and_create(&s, &mut token_map))
            .collect();

        let mut merges = Vec::new();

        let counts = Mutex::new(PairCounts::new());
        let counts_ref = &counts;

        {
            let mut h = counts.lock().unwrap();
            for s in strings.iter() {
                h.update(&token_map, s, 1);
            }
        }

        loop {
            // Find max pair or finish.
            let pair = if let Some((count, pair)) = counts.lock().unwrap().max() {
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

            let token_map_ref = &token_map;

            strings.par_chunks_mut(1000).for_each(move |chunk| {
                let mut local_delta = HashMap::new();
                let mut tmp = Tokens::default();
                for s in chunk {
                    if !s.contains(&bloom) {
                        continue;
                    }
                    if tmp.from_replace(token_map_ref, s, pair, merged) {
                        for p in s.pairs(token_map_ref) {
                            PairCounts::update_pair(&mut local_delta, p, -1);
                        }
                        for p in tmp.pairs(token_map_ref) {
                            PairCounts::update_pair(&mut local_delta, p, 1);
                        }
                        std::mem::swap(s, &mut tmp);
                    }
                }
                counts_ref.lock().unwrap().apply(local_delta);
            });
        }

        PairTokenizer { token_map, merges }
    }

    pub fn tokenize(&self, s: &str) -> Tokens {
        let mut tokens = Tokens::from_str_or_unknown(&s, &self.token_map);
        let mut tmp = Tokens::default();
        for (pair, merged) in self.merges.iter().cloned() {
            if tokens.contains(&pair) {
                if tmp.from_replace(&self.token_map, &tokens, pair, merged) {
                    std::mem::swap(&mut tmp, &mut tokens);
                }
            }
        }
        tokens
    }
}
