use crate::Error;
use crate::classifier::{
    Classifier, DirSizeClassifier, FileAgeClassifier, FileSizeClassifier, NaiveBayesClassifier,
};
use crate::ngrams::{Ngram, Ngrams};
use crate::normalize;
use crate::playlist::{M3uPlaylist, Playlist, PlaylistEntry};
use crate::tokenize::PairTokenizer;
use crate::tokens::{Token, Tokens};
use crate::viz;
use crate::vlc;
use crate::walk;
use crate::walk::Walk;
use crate::{BuildArgs, ScoreArgs};

use log::*;
use std::collections::HashSet;
use std::time::Instant;
use thread_priority::*;

#[derive(Debug)]
pub struct Entry {
    pub file: walk::File,
    pub normalized_path: String,
    pub tokens: Option<Tokens>,
    pub ngrams: Option<Ngrams>,
    pub scores: Box<[f64]>, // One score per classifier
}

pub struct App {
    common_args: crate::CommonArgs,
    vlc_args: Option<crate::VlcArgs>,
    batch_size: usize,
    include_classified: bool,
    dry_run: bool,
    entries: Vec<Entry>,
    tokenizer: Option<PairTokenizer>,
    frequent_ngrams: Option<ahash::AHashSet<Ngram>>,
    file_size_classifier: Option<FileSizeClassifier>,
    dir_size_classifier: Option<DirSizeClassifier>,
    file_age_classifier: Option<FileAgeClassifier>,
    naive_bayes: NaiveBayesClassifier,
    visualizer: viz::ScoreVisualizer,
    playlist: M3uPlaylist,
    vlc_controller: Option<vlc::VlcController>,
}

// Helper struct for timing
struct Timer {
    start: Instant,
    name: &'static str,
}

impl Timer {
    fn start(name: &'static str) -> Self {
        info!("Starting: {}", name);
        Timer {
            start: Instant::now(),
            name,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        info!("Finished: {} in {:?}", self.name, duration);
    }
}

// Macro for convenient timing
macro_rules! time_it {
    ($name:expr, $block:block) => {{
        let _timer = Timer::start($name);
        $block
    }};
}

impl App {
    pub fn new(build_args: BuildArgs, playlist: M3uPlaylist) -> Self {
        Self::new_common(
            build_args.common.clone(),
            Some(build_args.vlc.clone()),
            build_args.batch,
            false, // never include classified files for build command
            build_args.dry_run,
            playlist,
        )
    }

    pub fn new_for_scoring(score_args: ScoreArgs, playlist: M3uPlaylist) -> Self {
        Self::new_common(
            score_args.common.clone(),
            None,
            1, // batch_size not used for scoring
            score_args.include_classified,
            false,
            playlist,
        )
    }

    fn new_common(
        common_args: crate::CommonArgs,
        vlc_args: Option<crate::VlcArgs>,
        batch_size: usize,
        include_classified: bool,
        dry_run: bool,
        playlist: M3uPlaylist,
    ) -> Self {
        info!("{:#?}", common_args);

        // Initialize visualizer
        let visualizer = viz::ScoreVisualizer::default();

        // Initialize optional classifiers based on args
        let file_size_classifier = if let Some(log_base) = common_args.file_size.file_size_bias {
            assert!(log_base.abs() > 1.0, "File size log base must be > 1.0");
            let reverse = log_base < 0.0;
            Some(FileSizeClassifier::new(
                log_base.abs(),
                common_args.file_size.file_size_offset,
                reverse,
            ))
        } else {
            None
        };

        let dir_size_classifier = if let Some(log_base) = common_args.dir_size.dir_size_bias {
            assert!(
                log_base.abs() > 1.0,
                "Directory size log base must be > 1.0"
            );
            let reverse = log_base < 0.0;
            Some(DirSizeClassifier::new(
                log_base.abs(),
                common_args.dir_size.dir_size_offset,
                reverse,
            ))
        } else {
            None
        };

        let file_age_classifier = if let Some(log_base) = common_args.file_age.file_age_bias {
            assert!(log_base.abs() > 1.0, "File age log base must be > 1.0");
            let reverse = log_base < 0.0;
            Some(FileAgeClassifier::new(
                log_base.abs(),
                common_args.file_age.file_age_offset,
                reverse,
            ))
        } else {
            None
        };

        let vlc_controller = vlc_args
            .as_ref()
            .map(|args| vlc::VlcController::new(args.clone()));

        Self {
            common_args,
            vlc_args,
            batch_size,
            include_classified,
            dry_run,
            entries: Vec::new(),
            tokenizer: None,
            frequent_ngrams: None,
            file_size_classifier,
            dir_size_classifier,
            file_age_classifier,
            naive_bayes: NaiveBayesClassifier::new(false),
            visualizer,
            playlist,
            vlc_controller,
        }
    }

    fn set_threads_to_min_priority(&self) {
        rayon::broadcast(|_| {
            set_current_thread_priority(ThreadPriority::Min).unwrap();
        });
    }

    fn get_classifiers(&self) -> Vec<&dyn Classifier> {
        let mut classifiers = Vec::new();
        if let Some(ref classifier) = self.file_size_classifier {
            classifiers.push(classifier as &dyn Classifier);
        }
        if let Some(ref classifier) = self.dir_size_classifier {
            classifiers.push(classifier as &dyn Classifier);
        }
        if let Some(ref classifier) = self.file_age_classifier {
            classifiers.push(classifier as &dyn Classifier);
        }
        classifiers.push(&self.naive_bayes as &dyn Classifier);
        classifiers
    }

    fn collect_files(&mut self, include_classified: bool) {
        // Create set of already classified paths (convert relative paths to absolute)
        let mut classified_paths = HashSet::new();
        
        if !include_classified {
            let playlist_root = self.playlist.root();

            // Add all entries (both positive and negative) to the classified set
            for entry in self.playlist.entries() {
                let abs_path = playlist_root.join(entry.path());
                let normalized = normalize::normalize_path(&abs_path);
                classified_paths.insert(normalized);
            }
        }

        let walk = Walk::new(self.common_args.video_exts.iter().map(String::as_ref));
        for dir in &self.common_args.dirs {
            walk.walk_dir(dir);
        }

        let classifiers_len = self.get_classifiers().len();

        let file_receiver = walk.into_rx();
        while let Ok(file) = file_receiver.recv() {
            debug!("{:?}", file);

            let file_path = file.dir.join(&file.file_name);
            let abs_file_path = if file_path.is_absolute() {
                file_path.clone()
            } else {
                std::env::current_dir().unwrap().join(&file_path)
            };
            let normalized_file_path = normalize::normalize_path(&abs_file_path);

            // Skip if already classified (only when include_classified is false)
            if !include_classified && classified_paths.contains(&normalized_file_path) {
                debug!("Skipping already classified file: {:?}", file_path);
                continue;
            }

            let path_to_normalize = self.playlist.to_relative_path(&abs_file_path);
            let normalized_path = normalize::normalize(&path_to_normalize);

            let mut scores = vec![0.0; classifiers_len];
            scores.shrink_to_fit();

            // Initialize entry with scores array sized for all classifiers plus naive bayes
            let entry = Entry {
                file,
                normalized_path,
                tokens: None,
                ngrams: None,
                scores: scores.into_boxed_slice(),
            };

            // Update dir size classifier if present
            if let Some(ref mut dir_classifier) = self.dir_size_classifier {
                dir_classifier.add_entry(&entry);
            }

            self.entries.push(entry);
        }

        if include_classified {
            info!("Collected {} entries (including classified)", self.entries.len());
        } else {
            info!("Collected {} unclassified entries", self.entries.len());
        }
    }

    // Initializes the PairTokenizer by training it on all relevant paths
    fn initialize_tokenizer(&mut self) {
        // Collect all paths that need tokenization (candidates + playlist)
        let mut paths: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.normalized_path.to_string())
            .collect();

        // Add paths from playlist classifications
        paths.extend(self.playlist.entries().iter().map(|e| {
            let abs_path = self.playlist.root().join(e.path());
            let path_to_normalize = self.playlist.to_relative_path(&abs_path);
            normalize::normalize(&path_to_normalize)
        }));

        // Create tokenizer from all paths
        self.tokenizer = Some(PairTokenizer::new(paths.iter().map(String::as_str)));
        info!(
            "Tokenizer tokens {:?}",
            self.tokenizer.as_ref().unwrap().token_map().count()
        );
    }

    // Generates ngrams for all relevant paths, counts them, filters for frequent ones,
    // and stores tokens/ngrams for candidate entries.
    fn generate_ngrams(&mut self) {
        let tokenizer = self.tokenizer.as_ref().unwrap();

        // Collect all paths for ngram counting (candidates + playlist)
        let mut paths: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.normalized_path.to_string())
            .collect();
        paths.extend(self.playlist.entries().iter().map(|e| {
            let abs_path = self.playlist.root().join(e.path());
            let path_to_normalize = self.playlist.to_relative_path(&abs_path);
            normalize::normalize(&path_to_normalize)
        }));

        // Use the new function in ngrams.rs to count and filter
        self.frequent_ngrams = Some(Ngrams::count_and_filter_from_paths(
            &paths,
            tokenizer,
            self.common_args.windows,
        ));

        info!("total paths {:?}", paths.len());
        info!(
            "frequent ngrams {:?}",
            self.frequent_ngrams.as_ref().unwrap().len()
        );

        // Final pass to store tokens and frequent ngrams for candidates only
        for entry in self.entries.iter_mut() {
            // Tokenize the path and store the tokens
            entry.tokens = Some(tokenizer.tokenize(&entry.normalized_path));

            let mut ngrams = Ngrams::default();
            // Generate ngrams for the entry using the frequent filter
            ngrams.windows(
                entry.tokens.as_ref().unwrap(),
                self.common_args.windows,
                self.frequent_ngrams.as_ref(),
                None, // No debug info needed here
            );
            entry.ngrams = Some(ngrams);
        }
    }

    // Trains the NaiveBayesClassifier using the tokenized and ngramized playlist entries.
    fn train_naive_bayes_classifier(&mut self) {
        let tokenizer = self.tokenizer.as_ref().unwrap();
        let playlist_root = self.playlist.root();

        // Train naive bayes classifier on playlist entries
        let mut temp_ngrams = Ngrams::default();

        // Process all examples in a single loop
        for entry in self.playlist.entries().iter() {
            let path = entry.path();
            let abs_path = playlist_root.join(path);
            let path_to_normalize = self.playlist.to_relative_path(&abs_path);
            let normalized_path = normalize::normalize(&path_to_normalize);
            let tokens = tokenizer.tokenize(&normalized_path);
            // Original code used None for allowed ngrams during training
            temp_ngrams.windows(&tokens, self.common_args.windows, None, None);

            // Train based on entry type
            match entry {
                PlaylistEntry::Positive(_) => self.naive_bayes.train_positive(&temp_ngrams),
                PlaylistEntry::Negative(_) => self.naive_bayes.train_negative(&temp_ngrams),
            }
        }
    }

    fn calculate_scores_and_sort_entries(&mut self) {
        // Create temporary vector to swap with entries
        let mut temp_entries = Vec::new();
        std::mem::swap(&mut self.entries, &mut temp_entries);

        let classifiers = self.get_classifiers();

        // Calculate raw scores for each classifier
        for (idx, classifier) in classifiers.iter().enumerate() {
            for entry in &mut temp_entries {
                entry.scores[idx] = classifier.calculate_score(entry);
            }
        }

        // Normalize each column of scores
        for col in 0..classifiers.len() {
            let col_scores: Vec<f64> = temp_entries.iter().map(|e| e.scores[col]).collect();
            if let (Some(min), Some(max)) = (
                col_scores.iter().copied().reduce(f64::min),
                col_scores.iter().copied().reduce(f64::max),
            ) {
                if (max - min).abs() > f64::EPSILON {
                    for (entry, &raw_score) in temp_entries.iter_mut().zip(&col_scores) {
                        entry.scores[col] = (raw_score - min) / (max - min);
                    }
                }
            }
        }

        // Sort entries by total score ascending
        temp_entries.sort_by(|a, b| {
            let a_sum = a.scores.iter().sum::<f64>();
            let b_sum = b.scores.iter().sum::<f64>();
            a_sum.partial_cmp(&b_sum).expect("Invalid score comparison")
        });

        // Swap back the processed entries
        std::mem::swap(&mut self.entries, &mut temp_entries);
    }

    // Starts VLC and gets classification from user
    fn play_file_and_get_classification(&self, entry: &Entry) -> Option<vlc::Classification> {
        let path = entry.file.dir.join(&entry.file.file_name);
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir().unwrap().join(&path)
        };
        let file_name = Some(entry.file.file_name.to_string_lossy().to_string());

        // Send start playback to controller
        let vlc_controller = self
            .vlc_controller
            .as_ref()
            .expect("VLC controller required for classification");
        if let Err(e) = vlc_controller.start_playback(&abs_path, file_name) {
            error!("Failed to start VLC playback: {:?}", e);
            return None;
        }

        // Wait for classification with try_recv and sleep
        loop {
            match vlc_controller.try_recv_classification() {
                Ok(Some(classification)) => {
                    if matches!(classification, vlc::Classification::Skipped) {
                        error!("Classification skipped for {:?}", path);
                        return None;
                    } else {
                        return Some(classification);
                    }
                }
                Ok(None) => {
                    // No update yet, sleep and try again
                    std::thread::sleep(std::time::Duration::from_millis(
                        self.vlc_args.as_ref().unwrap().vlc_poll_interval,
                    ));
                }
                Err(e) => {
                    error!("Classification error: {:?}", e);
                    return None;
                }
            }
        }
    }

    // Displays detailed entry information including filename, tokens, and ngrams
    fn display_entry_details(&self, entry: &Entry) {
        let path = entry.file.dir.join(&entry.file.file_name);
        let token_map = self.tokenizer.as_ref().unwrap().token_map();

        // Display filename and normalized form
        println!("File: {:?}", path);
        let token_strs = entry.tokens.as_ref().unwrap().debug_strs(token_map);
        println!("Tokens: {:?}", token_strs);

        let mut ngram_tokens: Vec<Vec<Token>> = Vec::new();
        {
            let mut tmp_ngrams = Ngrams::default();
            tmp_ngrams.windows(
                entry.tokens.as_ref().unwrap(),
                self.common_args.windows,
                self.frequent_ngrams.as_ref(),
                Some(&mut ngram_tokens),
            );
            ngram_tokens.sort();
            ngram_tokens.dedup();
        }

        let mut ngram_scores = Vec::new();
        for window in ngram_tokens.into_iter() {
            let ngram = Ngram::new(&window);
            let score = self.naive_bayes.ngram_score(ngram);
            ngram_scores.push((window, score));
        }

        // Sort tuples by absolute score descending
        ngram_scores.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());

        // Display top 50 ngrams by absolute score
        println!("Top ngrams by absolute score:");
        for (tokens, score) in ngram_scores.iter().take(50) {
            let token_strs: Vec<&str> = tokens
                .iter()
                .map(|t| token_map.get_str(*t).unwrap())
                .collect();
            print!("{:?}: {:.3}, ", token_strs, score);
        }
        println!();

        // Display classifier scores
        let score_details: Vec<String> = self
            .get_classifiers()
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}: {:.3}", c.name(), entry.scores[i]))
            .collect();

        info!("Classifier scores: {}", score_details.join(", "));
    }

    // Updates classifiers and playlist with the classification result
    fn process_classification_result(
        &mut self,
        entry: Entry,
        classification: vlc::Classification,
    ) -> Result<(), Error> {
        let path = entry.file.dir.join(&entry.file.file_name);
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir().unwrap().join(&path)
        };

        // Update dir size classifier
        if let Some(ref mut dir_classifier) = self.dir_size_classifier {
            dir_classifier.remove_entry(&entry);
        }

        match classification {
            vlc::Classification::Positive => {
                self.playlist.add_positive(&abs_path)?;
                self.naive_bayes
                    .train_positive(entry.ngrams.as_ref().unwrap());
                info!("{:?} (POSITIVE)", path);
            }
            vlc::Classification::Negative => {
                self.playlist.add_negative(&abs_path)?;
                self.naive_bayes
                    .train_negative(entry.ngrams.as_ref().unwrap());
                info!("{:?} (NEGATIVE)", path);
            }
            vlc::Classification::Skipped => unreachable!(), // Handled in play_file_and_get_classification
        }

        Ok(())
    }

    // Handles the main classification loop
    fn classification_loop(&mut self) -> Result<(), Error> {
        while !self.entries.is_empty() {
            time_it!("Update classification scores", {
                self.calculate_scores_and_sort_entries();
            });

            let num_to_process = std::cmp::min(self.entries.len(), std::cmp::max(self.batch_size, 1));
            let entries_to_process: Vec<Entry> = self
                .entries
                .drain(self.entries.len() - num_to_process..)
                .collect();

            for entry in entries_to_process {
                // Get classifier names
                let classifier_names: Vec<&str> =
                    self.get_classifiers().iter().map(|c| c.name()).collect();

                // Display detailed information about the entry
                self.display_entry_details(&entry);

                // Display visualizations
                self.visualizer
                    .display_distributions(&self.entries, &entry, &classifier_names);

                if let Some(classification) = self.play_file_and_get_classification(&entry) {
                    self.process_classification_result(entry, classification)?;
                }
            }
        }
        Ok(())
    }

    // Main entry point remains simple and high-level
    pub fn run(&mut self) -> Result<(), Error> {
        self.set_threads_to_min_priority();

        // 1. Reading files (assuming this happens during walk_dir and collect_unclassified_files)
        time_it!("File Reading and Collection", {
            self.collect_files(self.include_classified);
        });

        time_it!("Tokenization", {
            self.initialize_tokenizer();
        });

        time_it!("Generate ngrams", {
            self.generate_ngrams();
        });

        time_it!("Train naive bayes classifier", {
            self.train_naive_bayes_classifier();
        });

        // Dry Run Check
        if self.dry_run {
            info!("Dry run enabled. Skipping classification loop.");
            return Ok(());
        }

        self.classification_loop()?;

        Ok(())
    }

    pub fn score_files(&mut self) -> Result<(), Error> {
        self.set_threads_to_min_priority();

        // Same initial steps as run() but without VLC classification loop
        time_it!("File Reading and Collection", {
            self.collect_files(self.include_classified);
        });

        time_it!("Tokenization", {
            self.initialize_tokenizer();
        });

        time_it!("Generate ngrams", {
            self.generate_ngrams();
        });

        time_it!("Train naive bayes classifier", {
            self.train_naive_bayes_classifier();
        });

        // Calculate scores and sort entries
        time_it!("Calculate scores", {
            self.calculate_scores_and_sort_entries();
        });

        // Display all files with their scores
        println!("Files ranked by classifier scores:");
        println!("{:60} {:>10}", "File", "Total Score");
        println!("{:-<71}", "");

        for entry in self.entries.iter().rev() {
            // Reverse to show highest scores first
            let path = entry.file.dir.join(&entry.file.file_name);
            let total_score: f64 = entry.scores.iter().sum();
            println!("{:60} {:>10.3}", path.display().to_string(), total_score);
        }

        Ok(())
    }
}
