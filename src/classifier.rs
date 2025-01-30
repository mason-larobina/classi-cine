use crate::ngrams::Ngram;
use crate::tokens::Tokens;
use std::path::Path;

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
    /// Returns a score indicating how likely the item should be kept
    fn score(&self, item: &ClassifierItem) -> Score;
}

/// Input data for classification
#[derive(Debug)]
pub struct ClassifierItem<'a> {
    /// Path to the file
    pub path: &'a Path,
    /// Normalized filename 
    pub norm: &'a str,
    /// Tokenized content
    pub tokens: Option<&'a Tokens>,
    /// Ngrams extracted from content
    pub ngrams: Option<&'a [Ngram]>,
}

/// Classifies based on file size
pub struct FileSizeClassifier {
    /// Base for logarithmic scaling
    log_base: f64,
}

impl FileSizeClassifier {
    pub fn new(log_base: f64) -> Self {
        Self { log_base }
    }
}

impl Classifier for FileSizeClassifier {
    fn score(&self, item: &ClassifierItem) -> Score {
        let size = std::fs::metadata(item.path)
            .map(|m| m.len())
            .unwrap_or(0);
        
        // Larger files get higher scores
        let score = if size == 0 {
            0.0
        } else {
            (size as f64).log(self.log_base) / 20.0 
        };
        
        Score::new(score)
    }
}

/// Classifies based on number of files in same directory
pub struct DirSizeClassifier;

impl Classifier for DirSizeClassifier {
    fn score(&self, item: &ClassifierItem) -> Score {
        let count = item.path.parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|entries| entries.count())
            .unwrap_or(0);

        // More files = higher score
        let score = (count as f64).log2() / 10.0;
        Score::new(score)
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
    fn score(&self, item: &ClassifierItem) -> Score {
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

    #[test]
    fn test_weighted_classifier() {
        let mut classifier = WeightedClassifier::new();
        
        // Add size and directory classifiers with weights
        classifier.add(FileSizeClassifier::new(2.0), 0.7);
        classifier.add(DirSizeClassifier, 0.3);

        let path = PathBuf::from("test.txt");
        let item = ClassifierItem {
            path: &path,
            norm: "test.txt",
            tokens: None,
            ngrams: None,
        };

        let score = classifier.score(&item);
        assert!(score.0 >= 0.0 && score.0 <= 1.0);
    }
}
