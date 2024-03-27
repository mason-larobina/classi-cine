use crate::tokens::{Token, Tokens};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Ngram(u64);

#[derive(Debug)]
pub struct Ngrams(Vec<Ngram>);

impl Ngram {
    fn new(tokens: &[Token]) -> Self {
        let mut hasher = DefaultHasher::new();
        tokens.hash(&mut hasher);
        Self(hasher.finish())
    }
}

pub fn generate_ngrams(
    tokens: &Tokens,
    windows: usize,
    out: &mut Vec<Ngram>,
    mut debug: Option<&mut Vec<Vec<Token>>>,
) {
    out.clear();

    for n in 1..=windows {
        for window in tokens.as_slice().windows(n) {
            out.push(Ngram::new(window));
            if let Some(d) = &mut debug {
                d.push(window.to_vec());
            }
        }
    }

    out.sort();
    out.dedup();
}

//pub fn ngrams(tokens_vec: &[Tokens], windows: usize) {}
