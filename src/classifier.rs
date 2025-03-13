use crate::Entry;
use crate::ngrams::{Ngram, Ngrams};
use log::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Trait for types that can classify files/content
pub trait Classifier {
    /// Returns the name of this classifier.
    fn name(&self) -> &'static str;

    /// Calculate score for an entry.
    fn calculate_score(&self, entry: &Entry) -> f64;
}

/// Classifies based on ngram frequencies in positive/negative examples
pub struct NaiveBayesClassifier {
    /// Ngram counts for positive examples
    positive_counts: HashMap<Ngram, u32>,
    /// Total positive examples seen
    positive_total: u64,
    /// Total positive ngrams seen
    positive_total_ngrams: u64,

    /// Ngram counts for negative examples
    negative_counts: HashMap<Ngram, u32>,
    /// Total negative examples seen
    negative_total: u64,
    /// Total negative ngrams seen
    negative_total_ngrams: u64,

    /// Set of all unique ngrams seen in either class
    vocabulary: HashSet<Ngram>,

    /// Whether to reverse the scoring
    reverse: bool,
}

impl NaiveBayesClassifier {
    pub fn ngram_score(&self, ngram: Ngram) -> f64 {
        let pos_prob = self.log_probability(ngram, true);
        let neg_prob = self.log_probability(ngram, false);
        pos_prob - neg_prob
    }

    pub fn new(reverse: bool) -> Self {
        Self {
            positive_counts: HashMap::new(),
            positive_total: 0,
            positive_total_ngrams: 0,
            negative_counts: HashMap::new(),
            negative_total: 0,
            negative_total_ngrams: 0,
            vocabulary: HashSet::new(),
            reverse,
        }
    }

    pub fn train_positive(&mut self, ngrams: &Ngrams) {
        self.positive_total += 1;
        for ngram in ngrams.iter() {
            *self.positive_counts.entry(*ngram).or_default() += 1;
            self.vocabulary.insert(*ngram);
            self.positive_total_ngrams += 1;
        }
    }

    pub fn train_negative(&mut self, ngrams: &Ngrams) {
        self.negative_total += 1;
        for ngram in ngrams.iter() {
            *self.negative_counts.entry(*ngram).or_default() += 1;
            self.vocabulary.insert(*ngram);
            self.negative_total_ngrams += 1;
        }
    }

    /// Returns log probability with Laplace smoothing
    fn log_probability(&self, ngram: Ngram, positive: bool) -> f64 {
        let (counts, total_ngrams) = if positive {
            (&self.positive_counts, self.positive_total_ngrams)
        } else {
            (&self.negative_counts, self.negative_total_ngrams)
        };
        // Laplace smoothing in log space
        let count = counts.get(&ngram).copied().unwrap_or(0) as f64;
        let vocab_size = self.vocabulary.len() as f64;
        ((1.0 + count) / (1.0 + total_ngrams as f64 + vocab_size)).ln()
    }
}

impl Classifier for NaiveBayesClassifier {
    fn name(&self) -> &'static str {
        "naive_bayes"
    }

    fn calculate_score(&self, item: &Entry) -> f64 {
        let ngrams = item.ngrams.as_ref().unwrap();

        // Calculate log probabilities for positive and negative cases using Bayes' theorem:
        // P(class|ngrams) ∝ P(ngrams|class) * P(class)
        // In log space this becomes:
        // log P(class|ngrams) = log P(ngrams|class) + log P(class) + const

        // Start with log prior probabilities: log P(class)
        // Priors account for class frequency in training data and prevent bias from unbalanced training
        // For example, if we have 90 positive and 10 negative examples:
        // log_positive = ln(0.9) ≈ -0.105
        // log_negative = ln(0.1) ≈ -2.302
        // This captures our prior belief that new examples are more likely to be positive
        let mut log_positive = ((1.0 + self.positive_total as f64)
            / (2 + self.positive_total + self.negative_total) as f64)
            .ln();
        let mut log_negative = ((1.0 + self.negative_total as f64)
            / (2 + self.positive_total + self.negative_total) as f64)
            .ln();

        // Add log likelihoods: log P(ngrams|class)
        for ngram in ngrams.iter() {
            log_positive += self.log_probability(*ngram, true);
            log_negative += self.log_probability(*ngram, false);
        }

        // Return difference of log probabilities
        // This maintains better numerical stability than converting to probabilities
        // Positive values indicate more likely positive, negative values more likely negative
        let score = log_positive - log_negative;
        assert!(score.is_finite());

        if self.reverse { -score } else { score }
    }
}

/// Classifies based on file size using logarithmic scaling
/// 
/// # Panics
/// Panics during construction if log_base <= 1.0
pub struct FileSizeClassifier {
    /// Base for logarithmic scaling (must be > 1.0)
    log_base: f64,
    /// Whether to reverse the scoring (larger files = lower score)
    reverse: bool,
}

impl FileSizeClassifier {
    pub fn new(log_base: f64, reverse: bool) -> Self {
        assert!(log_base > 1.0, "Log base must be greater than 1");
        Self { log_base, reverse }
    }
}

impl Classifier for FileSizeClassifier {
    fn name(&self) -> &'static str {
        "file_size"
    }

    fn calculate_score(&self, item: &Entry) -> f64 {
        let size = item.file.size;
        if size == 0 {
            0.0
        } else {
            let score = (size as f64).log(self.log_base);
            if !score.is_finite() {
                warn!("Invalid file size score for size {}: {}", size, score);
                0.0
            } else if self.reverse {
                -score
            } else {
                score
            }
        }
    }
}

/// Classifies based on number of files in same directory using logarithmic scaling
/// 
/// # Panics
/// Panics during construction if log_base <= 1.0
pub struct DirSizeClassifier {
    /// Base for logarithmic scaling (must be > 1.0)
    log_base: f64,
    /// Whether to reverse the scoring (more files = lower score)
    reverse: bool,
    /// Map of directory to file count
    dir_counts: HashMap<Arc<PathBuf>, usize>,
}

impl DirSizeClassifier {
    pub fn new(log_base: f64, reverse: bool) -> Self {
        assert!(log_base > 1.0, "Log base must be greater than 1");
        Self {
            log_base,
            reverse,
            dir_counts: HashMap::new(),
        }
    }

    pub fn add_entry(&mut self, entry: &Entry) {
        *self.dir_counts.entry(entry.file.dir.clone()).or_default() += 1;
    }

    pub fn remove_entry(&mut self, entry: &Entry) {
        if let std::collections::hash_map::Entry::Occupied(mut e) =
            self.dir_counts.entry(entry.file.dir.clone())
        {
            let count = e.get_mut();
            *count -= 1;
            if *count == 0 {
                e.remove();
            }
        }
    }
}

impl Classifier for DirSizeClassifier {
    fn name(&self) -> &'static str {
        "dir_size"
    }

    fn calculate_score(&self, item: &Entry) -> f64 {
        let count = self.dir_counts.get(&item.file.dir).copied().unwrap_or(0);
        let score = (count as f64).log(self.log_base);
        if !score.is_finite() {
            warn!("Invalid dir size score for count {}: {}", count, score);
            0.0
        } else if self.reverse {
            -score
        } else {
            score
        }
    }
}
