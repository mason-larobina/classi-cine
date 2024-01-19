//use crate::tokenizer::{Ngram, TokenizeMode, Tokenizer};
//use std::collections::HashMap;
//
//// The NgramCounter struct is designed to maintain counts of ngrams.
//#[derive(Debug)]
//pub struct NgramCounter {
//    // A HashMap storing the counts of each ngram.
//    counts: HashMap<Ngram, usize>,
//
//    // A running total of all ngrams observed.
//    total: usize,
//
//    unique_ngram_count: u32,
//}
//
////impl NgramCounter {
////    fn new(unique_ngram_count: u32) -> Self {
////        assert!(unique_ngram_count > 0);
////
////        Self {
////            counts: HashMap::new(),
////            total: 0,
////            unique_ngram_count,
////        }
////    }
////
////    // Increment the count for a given ngram.
////    fn inc(&mut self, ngram: Ngram) {
////        let e = self.counts.entry(ngram).or_default();
////        *e += 1;
////        self.total += 1;
////    }
////
////    // Get the smoothed log probability of observing a given ngram.
////    //
////    // Laplace smoothed.
////    fn log_p(&self, ngram: &Ngram) -> f64 {
////        let count = (self.counts.get(ngram).cloned().unwrap_or_default() + 1) as f64;
////        let total = (self.total + self.unique_ngram_count as usize) as f64;
////        (count / total).max(f64::MIN_POSITIVE).ln()
////    }
////}
//
//#[derive(Debug)]
//pub struct NaiveBayesClassifier {
//    delete: NgramCounter,
//    keep: NgramCounter,
//}
//
////impl NaiveBayesClassifier {
////    pub fn new(tokenizer: &Tokenizer) -> Self {
////        Self {
////            delete: NgramCounter::new(tokenizer),
////            keep: NgramCounter::new(tokenizer),
////        }
////    }
////
////    pub fn train_delete(&mut self, ngrams: &[Ngram]) {
////        for ngram in ngrams {
////            self.delete.inc(*ngram);
////        }
////    }
////
////    pub fn train_keep(&mut self, ngrams: &[Ngram]) {
////        for ngram in ngrams {
////            self.keep.inc(*ngram);
////        }
////    }
////
////    pub fn predict_delete(&self, ngrams: &[Ngram]) -> f64 {
////        let mut log_p = 0.0;
////        for ngram in ngrams {
////            log_p += self.delete.log_p(ngram);
////            log_p -= self.keep.log_p(ngram);
////        }
////        log_p
////    }
////
////    pub fn debug_delete(&self, tokenizer: &Tokenizer, ngrams: &[Ngram]) -> Vec<(f64, String)> {
////        let mut scores: Vec<(f64, String)> = Vec::new();
////
////        for ngram in ngrams {
////            let score = self.delete.log_p(ngram) - self.keep.log_p(ngram);
////
////            if let Some(tokens) = tokenizer.ngram_tokens.get(ngram) {
////                let mut v = Vec::new();
////                for token in tokens {
////                    if let Some(s) = tokenizer.token_string.get(token) {
////                        v.push(s.to_string());
////                    } else {
////                        v.push(String::from("*"));
////                    }
////                }
////
////                let k = match tokenizer.tokenize {
////                    TokenizeMode::Chars => v.join(""),
////                    TokenizeMode::Words => v.join(" "),
////                };
////
////                scores.push((score, k));
////            }
////        }
////
////        scores.sort_by(|a, b| a.partial_cmp(&b).unwrap());
////
////        for (k, _) in scores.iter_mut() {
////            *k = crate::round(*k);
////        }
////
////        scores
////    }
////}
