use crate::Entry;
use terminal_size::{Width, terminal_size};
use textplots::{Chart, Plot, Shape};

pub struct ScoreVisualizer {
    width: u32,
    height: u32,
}

impl Default for ScoreVisualizer {
    fn default() -> Self {
        // Get terminal width or use fallback
        let width = terminal_size()
            .map(|(Width(w), _)| w as u32 * 2 - 16)
            .unwrap_or(80);
        Self { width, height: 50 }
    }
}

impl ScoreVisualizer {
    pub fn display_distributions(
        &self,
        entries: &[Entry],
        current_entry: &Entry,
        classifier_names: &[&str],
    ) {
        for (idx, name) in classifier_names.iter().enumerate() {
            self.plot_distribution(name, entries, idx, current_entry.scores[idx]);
        }
    }

    fn plot_distribution(&self, name: &str, entries: &[Entry], idx: usize, current_score: f64) {
        let scores: Vec<(f32, f32)> = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i as f32, e.scores[idx] as f32))
            .collect();

        println!("\nScore distribution for {}:", name);

        let marker = vec![
            (0f32, current_score as f32),
            (entries.len() as f32, current_score as f32),
        ];

        Chart::new(self.width, self.height, 0.0, entries.len() as f32)
            .lineplot(&Shape::Lines(&scores))
            .lineplot(&Shape::Lines(&marker))
            .display();
    }
}
