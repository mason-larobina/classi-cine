use crate::tokens::{Token, Tokens};
use ahash::AHashSet;
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
}
