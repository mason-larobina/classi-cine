use crate::ngrams::{Ngram,Ngrams};
use crate::normalize;
use crate::tokens::Tokens;
use crate::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;
use log::*;

/// Trait for types that can classify files/content
pub trait Classifier {
    /// Returns the name of this classifier.
    fn name(&self) -> &'static str;

    /// Called when the list of entries changes (typically after classification and removal of an
    /// entry). Classifiers may choose to do work once when first called or each time it is called.
    fn process_entries(&mut self, entries: &[Entry]);
             
    /// Calculate score for an entry.
    fn calculate_score(&self, entry: &Entry) -> f64;
}

/// Classifies based on ngram frequencies in positive/negative examples
pub struct NaiveBayesClassifier {
    /// Ngram counts for positive examples
    positive_counts: HashMap<Ngram, u32>,
    /// Ngram counts for negative examples
    negative_counts: HashMap<Ngram, u32>,
    /// Total positive examples seen
    positive_total: u32,
    /// Total negative examples seen
    negative_total: u32,
    /// Whether to reverse the scoring
    reverse: bool,
}

impl NaiveBayesClassifier {
    pub fn new(reverse: bool) -> Self {
        Self {
            positive_counts: HashMap::new(),
            negative_counts: HashMap::new(),
            positive_total: 0,
            negative_total: 0,
            reverse,
        }
    }

    pub fn train_positive(&mut self, ngrams: &Ngrams) {
        self.positive_total += 1;
        for ngram in ngrams.iter() {
            *self.positive_counts.entry(*ngram).or_default() += 1;
        }
    }

    pub fn train_negative(&mut self, ngrams: &Ngrams) {
        self.negative_total += 1;
        for ngram in ngrams.iter() {
            *self.negative_counts.entry(*ngram).or_default() += 1;
        }
    }

    /// Returns log probability with Laplace smoothing
    fn log_probability(&self, ngram: &Ngram, positive: bool) -> f64 {
        let (counts, total) = if positive {
            (&self.positive_counts, self.positive_total)
        } else {
            (&self.negative_counts, self.negative_total)
        };

        // Laplace smoothing in log space
        let count = counts.get(ngram).copied().unwrap_or(0) as f64;
        let vocab_size = self.positive_counts.len() + self.negative_counts.len();
        ((1.0 + count) / (1.0 + (total as f64) + (vocab_size as f64))).ln()
    }
}

impl Classifier for NaiveBayesClassifier {
    fn name(&self) -> &'static str {
        "naive_bayes"
    }

    fn process_entries(&mut self, _entries: &[Entry]) {
        // Training happens with the train_positive and train_negative functions.
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
        let mut log_positive = ((1.0 + self.positive_total as f64) / 
            (1 + self.positive_total + self.negative_total) as f64).ln();
        let mut log_negative = (1.0 + self.negative_total as f64 / 
            (1 + self.positive_total + self.negative_total) as f64).ln();

        dbg!(log_positive);
        dbg!(log_negative);

        // Add log likelihoods: log P(ngrams|class)
        for ngram in ngrams.iter() {
            log_positive += self.log_probability(ngram, true);
            log_negative += self.log_probability(ngram, false);
        }

        // Return difference of log probabilities
        // This maintains better numerical stability than converting to probabilities
        // Positive values indicate more likely positive, negative values more likely negative
        let score = log_positive - log_negative;
        
        // Check for invalid values
        if !score.is_finite() {
            warn!("Invalid naive bayes score: {} (log_pos={}, log_neg={})", 
                  score, log_positive, log_negative);
            0.0 // Return neutral score for invalid calculations
        } else if self.reverse {
            -score
        } else {
            score
        }
    }
}

/// Classifies based on file size
pub struct FileSizeClassifier {
    /// Base for logarithmic scaling
    log_base: f64,
    /// Whether to reverse the scoring (larger files = lower score)
    reverse: bool,
    /// Minimum log size seen
    min_log_size: f64,
    /// Maximum log size seen 
    max_log_size: f64,
}

impl FileSizeClassifier {
    pub fn new(log_base: f64, reverse: bool) -> Self {
        Self { 
            log_base,
            reverse,
            min_log_size: f64::MAX,
            max_log_size: f64::MIN,
        }
    }
}

impl Classifier for FileSizeClassifier {
    fn name(&self) -> &'static str {
        "file_size"
    }

    fn process_entries(&mut self, entries: &[Entry]) {
        for item in entries {
            let size = item.file.size;
            let log_score = if size == 0 {
                0.0
            } else {
                (size as f64).log(self.log_base)
            };
            self.min_log_size = self.min_log_size.min(log_score);
            self.max_log_size = self.max_log_size.max(log_score);
        }
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

/// Classifies based on number of files in same directory
pub struct DirSizeClassifier {
    /// Base for logarithmic scaling
    log_base: f64,
    /// Whether to reverse the scoring (more files = lower score)
    reverse: bool,
    /// Minimum log count seen
    min_log_count: f64,
    /// Maximum log count seen
    max_log_count: f64,
    /// Map of directory to file count
    dir_counts: std::collections::HashMap<Arc<PathBuf>, usize>,
}

impl DirSizeClassifier {
    pub fn new(log_base: f64, reverse: bool) -> Self {
        Self { 
            log_base,
            reverse,
            min_log_count: f64::MAX,
            max_log_count: f64::MIN,
            dir_counts: std::collections::HashMap::new(),
        }
    }
}

impl Classifier for DirSizeClassifier {
    fn name(&self) -> &'static str {
        "dir_size"
    }

    fn process_entries(&mut self, entries: &[Entry]) {
        // Count files per directory
        self.dir_counts.clear();
        for item in entries {
            *self.dir_counts.entry(item.file.dir.clone()).or_default() += 1;
        }
        
        // Calculate bounds from directory counts
        for count in self.dir_counts.values() {
            let log_score = (*count as f64).log(self.log_base);
            self.min_log_count = self.min_log_count.min(log_score);
            self.max_log_count = self.max_log_count.max(log_score);
        }
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

