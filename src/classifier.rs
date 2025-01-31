use crate::ngrams::Ngram;
use crate::tokens::Tokens;
use crate::Entry;
use std::path::{Path, PathBuf};

/// A score between 0.0 and 1.0 indicating classification confidence
#[derive(Debug, Clone, Copy)]
pub struct Score(pub f64);

impl Score {
    pub fn new(value: f64) -> Self {
        Self(value.clamp(0.0, 1.0))
    }
}

/// Trait for types that can classify files/content
pub trait Classifier {
    /// Process all entries to calculate scoring bounds
    fn process_bounds(&mut self, entries: &[Entry]);
    
    /// Returns a score indicating how likely the item should be kept
    fn score(&self, item: &Entry) -> Score;
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

    fn normalize(&self, score: f64) -> f64 {
        // Normalize to 0.0-1.0 range
        if (self.max_log_size - self.min_log_size).abs() < f64::EPSILON {
            0.5 // If min==max, return middle value
        } else {
            (score - self.min_log_size) / (self.max_log_size - self.min_log_size)
        }
    }
}

impl Classifier for FileSizeClassifier {
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

    fn score(&self, item: &Entry) -> Score {
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
        
        Score::new(final_score)
    }
}

/// Classifies based on number of files in same directory
pub struct DirSizeClassifier {
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
    pub fn new(reverse: bool) -> Self {
        Self { 
            reverse,
            min_log_count: f64::MAX,
            max_log_count: f64::MIN,
            dir_counts: std::collections::HashMap::new(),
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
}

impl Classifier for DirSizeClassifier {
    fn process_bounds(&mut self, entries: &[Entry]) {
        // Count files per directory
        self.dir_counts.clear();
        for item in entries {
            *self.dir_counts.entry(item.file.dir.clone()).or_default() += 1;
        }
        
        // Calculate bounds from directory counts
        for count in self.dir_counts.values() {
            let log_score = (*count as f64).log2();
            self.min_log_count = self.min_log_count.min(log_score);
            self.max_log_count = self.max_log_count.max(log_score);
        }
    }

    fn score(&self, item: &Entry) -> Score {
        // Use cached directory count
        let count = self.dir_counts.get(&item.file.dir).copied().unwrap_or(0);
        let log_score = (count as f64).log2();
        let normalized = self.normalize(log_score);
        
        // Reverse if requested
        let final_score = if self.reverse {
            1.0 - normalized
        } else {
            normalized
        };
        
        Score::new(final_score)
    }
}

/// Combines multiple classifiers with weights
pub struct WeightedClassifier {
    classifiers: Vec<(Box<dyn Classifier>, f64)>, 
}

impl WeightedClassifier {
    pub fn new() -> Self {
        Self {
            classifiers: Vec::new()
        }
    }

    pub fn add(&mut self, classifier: impl Classifier + 'static, weight: f64) {
        self.classifiers.push((Box::new(classifier), weight));
    }
}

impl Classifier for WeightedClassifier {
    fn process_bounds(&mut self, entries: &[Entry]) {
        for (classifier, _) in self.classifiers.iter_mut() {
            classifier.process_bounds(entries);
        }
    }

    fn score(&self, item: &Entry) -> Score {
        let mut total_score = 0.0;
        let mut total_weight = 0.0;

        for (classifier, weight) in &self.classifiers {
            total_score += classifier.score(item).0 * weight;
            total_weight += weight;
        }

        Score::new(total_score / total_weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn test_weighted_classifier() {
        let mut classifier = WeightedClassifier::new();
        
        // Add size and directory classifiers with weights
        classifier.add(FileSizeClassifier::new(2.0, false), 0.7);
        classifier.add(DirSizeClassifier::new(false), 0.3);

        use crate::walk::File;
        let file = File {
            dir: Arc::new(PathBuf::from(".")),
            file_name: "test.txt".into(),
            size: 1000,
            inode: 1000
        };
        let entry = Entry {
            file,
            norm: "test.txt".into(),
            tokens: None,
            ngrams: None,
        };

        let score = classifier.score(&entry);
        assert!(score.0 >= 0.0 && score.0 <= 1.0);
    }
}
