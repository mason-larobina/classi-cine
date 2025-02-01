use crate::ngrams::Ngram;
use crate::normalize;
use crate::tokens::Tokens;
use crate::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::collections::HashMap;

/// Trait for types that can classify files/content
pub trait Classifier {
    /// Returns the name of this classifier
    fn name(&self) -> &'static str;

    /// Process all entries to calculate scoring bounds
    fn process_bounds(&mut self, entries: &[Entry]);
    
    /// Normalize a raw score to 0.0-1.0 range
    fn normalize(&self, score: f64) -> f64;
    
    /// Returns a score between 0.0 and 1.0 indicating how likely the item should be kept
    fn score(&self, item: &Entry) -> f64;
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

    fn count_ngrams(&self, ngrams: &Ngrams) -> HashMap<Ngram, u32> {
        let mut counts = HashMap::new();
        for ngram in ngrams.iter() {
            *counts.entry(*ngram).or_default() += 1;
        }
        counts
    }

    fn train_positive(&mut self, ngrams: &Ngrams) {
        self.positive_total += 1;
        for (ngram, count) in self.count_ngrams(ngrams) {
            *self.positive_counts.entry(ngram).or_default() += count;
        }
    }

    fn train_negative(&mut self, ngrams: &Ngrams) {
        self.negative_total += 1;
        for (ngram, count) in self.count_ngrams(ngrams) {
            *self.negative_counts.entry(ngram).or_default() += count;
        }
    }

    fn probability(&self, ngram: &Ngram, positive: bool) -> f64 {
        let (counts, total) = if positive {
            (&self.positive_counts, self.positive_total)
        } else {
            (&self.negative_counts, self.negative_total)
        };

        // Laplace smoothing
        let count = counts.get(ngram).copied().unwrap_or(0) as f64;
        let vocab_size = self.positive_counts.len() + self.negative_counts.len();
        let probability = (count + 1.0) / ((total as f64) + (vocab_size as f64));
        probability.max(f64::MIN_POSITIVE).ln()
    }
}

impl Classifier for NaiveBayesClassifier {
    fn name(&self) -> &'static str {
        "naive_bayes"
    }

    fn process_bounds(&mut self, entries: &[Entry]) {
        // Clear previous training
        self.positive_counts.clear();
        self.negative_counts.clear();
        self.positive_total = 0;
        self.negative_total = 0;

        // Train on playlist entries
        let tokenizer = entries[0].tokens.as_ref().unwrap().tokenizer();
        let mut temp_ngrams = Ngrams::default();

        // Process positive examples
        for path in tokenizer.playlist().positives() {
            let norm = normalize::normalize(path);
            let tokens = tokenizer.tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            self.train_positive(&temp_ngrams);
        }

        // Process negative examples
        for path in tokenizer.playlist().negatives() {
            let norm = normalize::normalize(path);
            let tokens = tokenizer.tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            self.train_negative(&temp_ngrams);
        }
    }

    fn normalize(&self, score: f64) -> f64 {
        // Score is already normalized by the naive bayes calculation
        score
    }

    fn score(&self, item: &Entry) -> f64 {
        let ngrams = item.ngrams.as_ref().unwrap();
        
        // Calculate log probabilities to avoid underflow
        let mut positive_prob = (self.positive_total as f64 / 
            (self.positive_total + self.negative_total) as f64).ln();
        let mut negative_prob = (self.negative_total as f64 / 
            (self.positive_total + self.negative_total) as f64).ln();

        for ngram in ngrams.iter() {
            positive_prob += self.probability(ngram, true).ln();
            negative_prob += self.probability(ngram, false).ln();
        }

        // Convert back from log space and normalize
        let pos_exp = positive_prob.exp();
        let neg_exp = negative_prob.exp();
        let normalized = pos_exp / (pos_exp + neg_exp);

        if self.reverse {
            1.0 - normalized
        } else {
            normalized
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

    fn process_bounds(&mut self, entries: &[Entry]) {
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

    fn normalize(&self, score: f64) -> f64 {
        // Normalize to 0.0-1.0 range
        if (self.max_log_size - self.min_log_size).abs() < f64::EPSILON {
            0.5 // If min==max, return middle value
        } else {
            (score - self.min_log_size) / (self.max_log_size - self.min_log_size)
        }
    }

    fn score(&self, item: &Entry) -> f64 {
        let size = item.file.size;
        let log_score = if size == 0 {
            0.0
        } else {
            (size as f64).log(self.log_base)
        };
        
        let normalized = self.normalize(log_score);
        
        // Reverse if requested
        let final_score = if self.reverse {
            1.0 - normalized
        } else {
            normalized
        };
        
        final_score.clamp(0.0, 1.0)
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

    fn process_bounds(&mut self, entries: &[Entry]) {
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

    fn normalize(&self, score: f64) -> f64 {
        // Normalize to 0.0-1.0 range
        if (self.max_log_count - self.min_log_count).abs() < f64::EPSILON {
            0.5 // If min==max, return middle value
        } else {
            (score - self.min_log_count) / (self.max_log_count - self.min_log_count)
        }
    }

    fn score(&self, item: &Entry) -> f64 {
        // Use cached directory count
        let count = self.dir_counts.get(&item.file.dir).copied().unwrap_or(0);
        let log_score = (count as f64).log(self.log_base);
        let normalized = self.normalize(log_score);
        
        // Reverse if requested
        let final_score = if self.reverse {
            1.0 - normalized
        } else {
            normalized
        };
        
        final_score.clamp(0.0, 1.0)
    }
}

