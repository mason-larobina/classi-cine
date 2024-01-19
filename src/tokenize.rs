use crate::tokens::*;
use log::*;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
struct PairCounts {
    counts: HashMap<(Token, Token), i64>,
}

impl PairCounts {
    fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    fn update(&mut self, tokens: &Tokens, delta: i64, max_token_len: u32) {
        for (a, b) in tokens.pairs() {
            if a.is_special() || b.is_special() || a.len() + b.len() > max_token_len {
                continue;
            }
            let e = self.counts.entry((a, b)).or_default();
            *e += delta;
        }
    }

    fn max(&self) -> Option<(i64, Token, Token)> {
        let mut top = None;
        for ((a, b), count) in self.counts.iter() {
            let new = Some((*count, *a, *b));
            if top.is_none() || top < new {
                top = new;
            }
        }
        top
    }

    fn update_from(&mut self, other: PairCounts) {
        for ((a, b), delta) in other.counts {
            let e = self.counts.entry((a, b)).or_default();
            *e += delta;
        }
    }
}

pub fn subwords(files: &[String], chunk_size: usize, max_token_len: u32) -> (Vocab, Vec<Tokens>) {
    assert!(!files.is_empty());
    info!("File count: {}", files.len());

    // Init vocab and add special tokens.
    let mut vocab = Vocab::new();
    vocab.insert_special(" ");
    vocab.insert_special(std::path::MAIN_SEPARATOR_STR);

    let mut tokens_vec: Vec<Tokens> = Vec::with_capacity(files.len());
    let mut pair_counts = PairCounts::new();

    for file in files {
        // Build vocab and inital tokens vec.
        let mut tokens = Tokens::new();
        let mut s = String::new();
        for c in file.chars() {
            s.clear();
            s.push(c);
            let token = vocab.insert(&s);
            tokens.push(token);
        }

        pair_counts.update(&tokens, 1, max_token_len);

        tokens_vec.push(tokens);
    }

    while let Some((count, a, b)) = pair_counts.max() {
        if count < 2 {
            break;
        }

        let a_str = vocab.get_str(a);
        let b_str = vocab.get_str(b);
        let c_str = format!("{}{}", a_str, b_str);
        debug!("Merge {:?} {:?} -> {:?} ({}/2)", a_str, b_str, c_str, count);
        let c = vocab.insert(&c_str);

        let pair_counts_updates: Vec<_> = tokens_vec
            .par_chunks_mut(chunk_size)
            .map(move |chunk| {
                let mut new_tokens: Tokens = Tokens::new();
                let mut pair_counts_update = PairCounts::new();
                for tokens in chunk.iter_mut() {
                    tokens.replace_new(a, b, c, &mut new_tokens);
                    if tokens.len() == new_tokens.len() {
                        continue;
                    }
                    pair_counts_update.update(&tokens, -1, max_token_len);
                    pair_counts_update.update(&new_tokens, 1, max_token_len);
                    tokens.swap(&mut new_tokens);
                }
                pair_counts_update
            })
            .collect();

        for update in pair_counts_updates {
            pair_counts.update_from(update);
        }
    }

    (vocab, tokens_vec)
}

#[test]
fn subwords_test() {
    let files = vec![String::from("/apple/b c/d")];
    let (vocab, tokens_vec) = chars(&files);
    assert_eq!(files.len(), tokens_vec.len());
    let tokens = &tokens_vec[0];
    let mut tokens_str = Vec::new();
    for token in tokens.as_slice() {
        tokens_str.push(vocab.get_str(*token));
    }
    assert_eq!(
        tokens_str,
        vec!["/", "a", "p", "p", "l", "e", "/", "b", " ", "c", "/", "d"]
    );
}

pub fn chars(files: &[String]) -> (Vocab, Vec<Tokens>) {
    let mut vocab = Vocab::new();
    let mut tokens_vec: Vec<Tokens> = Vec::with_capacity(files.len());
    for file in files {
        let mut tokens = Tokens::new();
        let mut s = String::new();
        for c in file.chars() {
            s.clear();
            s.push(c);
            let token = vocab.insert_special(&s);
            tokens.push(token);
        }
        tokens_vec.push(tokens);
    }
    (vocab, tokens_vec)
}

#[test]
fn chars_test() {
    let files = vec![String::from("/apple/b c/d")];
    let (vocab, tokens_vec) = chars(&files);
    assert_eq!(files.len(), tokens_vec.len());
    let tokens = &tokens_vec[0];
    let mut tokens_str = Vec::new();
    for token in tokens.as_slice() {
        tokens_str.push(vocab.get_str(*token));
    }
    assert_eq!(
        tokens_str,
        vec!["/", "a", "p", "p", "l", "e", "/", "b", " ", "c", "/", "d"]
    );
}

pub fn words(files: &[String]) -> (Vocab, Vec<Tokens>) {
    let mut vocab = Vocab::new();
    let slash = vocab.insert_special(std::path::MAIN_SEPARATOR_STR);
    let mut tokens_vec: Vec<Tokens> = Vec::with_capacity(files.len());
    for file in files {
        let mut tokens = Tokens::new();
        for (i, comp) in file.split(std::path::MAIN_SEPARATOR).enumerate() {
            if i > 0 {
                tokens.push(slash);
            }
            for word in comp.split(" ") {
                if !word.is_empty() {
                    tokens.push(vocab.insert_special(word));
                }
            }
        }
        tokens_vec.push(tokens);
    }
    (vocab, tokens_vec)
}

#[test]
fn words_test() {
    let files = vec![String::from("/apple/b c/d")];
    let (vocab, tokens_vec) = words(&files);
    assert_eq!(files.len(), tokens_vec.len());
    let tokens = &tokens_vec[0];
    let mut tokens_str = Vec::new();
    for token in tokens.as_slice() {
        tokens_str.push(vocab.get_str(*token));
    }
    assert_eq!(tokens_str, vec!["/", "apple", "/", "b", "c", "/", "d"]);
}
