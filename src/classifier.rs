use crate::ngrams::Ngram;
use crate::tokens::Tokens;
use crate::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

