use log::*;
use textplots::{Chart, Plot, Shape};
use crate::Entry;

pub struct ScoreVisualizer {
    width: u32,
    height: u32,
}

impl Default for ScoreVisualizer {
    fn default() -> Self {
        Self {
            width: 300,
            height: 50,
        }
    }
}

impl ScoreVisualizer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn display_distributions(&self, entries: &[Entry], current_entry: &Entry, classifier_names: &[&str]) {
        for (idx, name) in classifier_names.iter().enumerate() {
            self.plot_distribution(name, entries, idx, current_entry.scores[idx]);
        }
    }

    pub fn display_score_details(&self, entry: &Entry, classifier_names: &[&str]) {
        let score_details: Vec<String> = entry.scores.iter()
            .enumerate()
            .map(|(i, score)| format!("{}: {:.3}", classifier_names[i], score))
            .collect();
        
        let path = entry.file.dir.join(&entry.file.file_name);
        info!("Top candidate: {:?}\nScores: {}", path, score_details.join(", "));
    }

    fn plot_distribution(&self, name: &str, entries: &[Entry], idx: usize, current_score: f64) {
        let scores: Vec<(f32, f32)> = entries.iter()
            .enumerate()
            .map(|(i, e)| (i as f32, e.scores[idx] as f32))
            .collect();
        
        println!("\nScore distribution for {}:", name);
        
        let marker = vec![(0f32, current_score as f32), (entries.len() as f32, current_score as f32)];
        
        Chart::new(self.width, self.height, 0.0, entries.len() as f32)
            .lineplot(&Shape::Lines(&scores))
            .lineplot(&Shape::Lines(&marker))
            .display();
    }
}
