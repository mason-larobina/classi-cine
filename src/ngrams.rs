use crate::tokens::{Token, Tokens};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Ngram(u64);

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
    pub fn generate(&mut self, tokens: &Tokens, windows: usize, mut debug: Option<&mut Vec<Vec<Token>>>) {
        self.0.clear();
        for n in 1..=windows {
            for window in tokens.as_slice().windows(n) {
                self.0.push(Ngram::new(window));
                if let Some(d) = &mut debug {
                    d.push(window.to_vec());
                }
            }
        }
        self.0.sort();
        self.0.dedup();
    }
}