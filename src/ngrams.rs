use crate::tokens::{Token, Tokens};
use ahash::AHashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Copy, Clone)]
pub struct Ngram(pub(crate) u64);

impl Ngram {
    pub fn debug_str(&self, tokens: &Tokens) -> String {
        let slice = tokens.as_slice();
        let mut result = String::new();
        let mut hasher = DefaultHasher::new();
        
        // Try different window sizes to find matching hash
        for n in 1..=5 {
            for window in slice.windows(n) {
                window.hash(&mut hasher);
                if hasher.finish() == self.0 {
                    return window.iter()
                        .map(|t| t.0.to_string())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                hasher = DefaultHasher::new();
            }
        }
        result
    }
}

#[derive(Debug, Default)]
pub struct Ngrams(Vec<Ngram>);

impl Ngram {
    fn new(tokens: &[Token]) -> Self {
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
