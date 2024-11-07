use crate::tokens::*;
use log::*;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct PairCounts {
    counts: HashMap<Pair, i64>,
}

impl PairCounts {
    fn update(&mut self, tokens: &Tokens, delta: i64) {
        if delta == 0 {
            return;
        }
        for pair in tokens.pairs() {
            match self.counts.entry(pair) {
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

    fn update_from(&mut self, other: PairCounts) {
        for (pair, delta) in other.counts {
            let e = self.counts.entry(pair).or_default();
            *e += delta;
        }
    }

    fn max(&self) -> Option<(i64, Pair)> {
        let mut top = None;
        for (pair, count) in self.counts.iter() {
            let new = Some((*count, *pair));
            if top.is_none() || top < new {
                top = new;
            }
        }
        top
    }
}

#[derive(Debug)]
pub struct PairTokenizer {
    token_map: TokenMap,
    unknown: Token,
    path_sep: Token,
    merges: Vec<(Token, Token, Token)>,
}

impl PairTokenizer {
    pub fn new(strings: Vec<String>) -> PairTokenizer {
        let mut token_map = TokenMap::default();
        let unknown = token_map.create_token("<UNKNOWN>");
        let path_sep = token_map.create_token(std::path::MAIN_SEPARATOR_STR);

        assert!(strings.len() > 0);
        let limit: f64 = strings.len() as f64;
        let limit: i64 = limit.log2() as i64;
        info!("limit {:?}", limit);

        let mut strings: Vec<Tokens> = strings.into_iter().map(|s| Tokens::from_str(&s, &mut token_map)).collect();

        let mut merges = Vec::new();
        let mut counts = PairCounts::default();
        let mut tmp = Tokens::default();

        for s in strings.iter() {
            counts.update(s, 1);
        }

        loop {
            // Find max pair or finish.
            let pair = if let Some((count, pair)) = counts.max() {
                if count < limit {
                    break;
                }
                info!("{:?} {:?}", count, pair.to_string(&token_map));
                pair
            } else {
                break;
            };

            // Merge pair into new token.
            let new = token_map.merge_create_token(pair);
            merges.push((pair.0, pair.1, new));

            // Apply merges and update counts.
            for s in strings.iter_mut() {
                if s.contains(&pair) {
                    if tmp.from_replace(s, pair, new) {
                        counts.update(s, -1);
                        counts.update(&tmp, 1);
                        std::mem::swap(&mut tmp, s);
                    }
                }
            }
        }

        PairTokenizer {
            token_map,
            unknown,
            path_sep,
            merges,
        }
    }
}
