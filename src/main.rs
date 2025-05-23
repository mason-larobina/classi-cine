mod bloom;
mod classifier;
mod ngrams;
mod normalize;
mod playlist;
mod tokenize;
mod tokens;
mod viz;
mod vlc;
mod walk;

use crate::ngrams::{Ngram, Ngrams};
use crate::playlist::{M3uPlaylist, Playlist, PlaylistEntry};
use crate::tokenize::PairTokenizer;
use crate::tokens::{Token, Tokens};
use crate::walk::Walk;
use clap::{Parser, Subcommand};
use classifier::{
    Classifier, DirSizeClassifier, FileAgeClassifier, FileSizeClassifier, NaiveBayesClassifier,
};
use log::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;
use thread_priority::*;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP request failed: {0}")]
    Reqwest(
        #[from]
        #[source]
        reqwest::Error,
    ),

    #[error("JSON parsing failed: {0}")]
    SerdeJson(
        #[from]
        #[source]
        serde_json::Error,
    ),

    #[error("I/O error: {0}")]
    Io(
        #[from]
        #[source]
        std::io::Error,
    ),

    #[error("Operation timed out: {0}")]
    Timeout(String),

    #[error("Filename mismatch - expected: {expected}, got: {got}")]
    FilenameMismatch { expected: String, got: String },

    #[error("VLC process failed: {0}")]
    ProcessFailed(#[source] std::io::Error),

    #[error("Failed to bind to port: {0}")]
    PortBindingFailed(#[source] std::io::Error),

    #[error("VLC not responding: {0}")]
    VLCNotResponding(String),

    #[error("Playlist error: {0}")]
    PlaylistError(String),

    #[error("File walk error: {0}")]
    WalkError(String),
}

#[derive(Parser, Debug, Clone)]
#[command(name = "classi-cine")]
struct Args {
    #[command(subcommand)]
    command: Command,

    #[clap(long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Build playlist through interactive classification
    Build(BuildArgs),
    /// List classified files
    ListPositive(ListArgs),
    ListNegative(ListArgs),
    /// Move playlist to a new location and rebase paths
    Move(MoveArgs),
}

#[derive(Parser, Debug, Clone)]
struct MoveArgs {
    /// Original M3U playlist file
    original: PathBuf,
    /// New M3U playlist file location
    new: PathBuf,
}

#[derive(Parser, Debug, Clone)]
struct BuildArgs {
    /// M3U playlist file for storing classifications
    playlist: PathBuf,
    /// Directories to scan for video files
    dirs: Vec<PathBuf>,
    /// Video file extensions to scan for
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4"
    )]
    video_exts: Vec<String>,
    #[clap(long, default_value_t = 5)]
    windows: usize,
    #[command(flatten)]
    vlc: VlcArgs,
    #[command(flatten)]
    file_size: FileSizeArgs,
    #[command(flatten)]
    dir_size: DirSizeArgs,
    #[command(flatten)]
    file_age: FileAgeArgs,
    /// Number of top entries to classify in each iteration
    #[clap(long, default_value_t = 1)]
    top_n: usize,
    /// Perform all steps except opening and classifying files.
    #[clap(long)]
    dry_run: bool,
}

#[derive(Parser, Debug, Clone)]
pub struct VlcArgs {
    /// Fullscreen VLC playback
    #[clap(long)]
    fullscreen: bool,
    /// Timeout in seconds for VLC startup
    #[clap(long, default_value_t = 60)]
    vlc_timeout: u64,
    /// Polling interval in milliseconds for VLC status checks
    #[clap(long, default_value_t = 100)]
    vlc_poll_interval: u64,
}

#[derive(Parser, Debug, Clone)]
struct FileSizeArgs {
    /// Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias.
    #[clap(long)]
    file_size_bias: Option<f64>,
    /// Offset to add to file size before log scaling.
    #[clap(long, default_value_t = 1048576)]
    file_size_offset: u64,
}

#[derive(Parser, Debug, Clone)]
struct DirSizeArgs {
    /// Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias.
    #[clap(long)]
    dir_size_bias: Option<f64>,
    /// Offset to add to directory size before log scaling.
    #[clap(long, default_value_t = 0)]
    dir_size_offset: usize,
}

#[derive(Parser, Debug, Clone)]
struct FileAgeArgs {
    /// Bias scoring based on file age (log base, > 1.0). Negative reverses bias (older files get higher score).
    #[clap(long)]
    file_age_bias: Option<f64>,
    /// Offset to add to file age in seconds before log scaling.
    #[clap(long, default_value_t = 86400)]
    file_age_offset: u64,
}

#[derive(Parser, Debug, Clone)]
struct ListArgs {
    /// M3U playlist file
    playlist: PathBuf,
}

#[derive(Debug)]
struct Entry {
    file: walk::File,
    normalized_path: String,
    tokens: Option<Tokens>,
    ngrams: Option<Ngrams>,
    scores: Box<[f64]>, // One score per classifier
}

struct Build {
    build_args: BuildArgs,
    entries: Vec<Entry>,
    tokenizer: Option<PairTokenizer>,
    frequent_ngrams: Option<ahash::AHashSet<Ngram>>,
    file_size_classifier: Option<FileSizeClassifier>,
    dir_size_classifier: Option<DirSizeClassifier>,
    file_age_classifier: Option<FileAgeClassifier>,
    naive_bayes: NaiveBayesClassifier,
    visualizer: viz::ScoreVisualizer,
    playlist: M3uPlaylist,
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

impl Build {
    fn new(build_args: BuildArgs, playlist: M3uPlaylist) -> Self {
        info!("{:#?}", build_args);

        // Initialize visualizer
        let visualizer = viz::ScoreVisualizer::default();

        // Initialize optional classifiers based on args
        let file_size_classifier = if let Some(log_base) = build_args.file_size.file_size_bias {
            assert!(log_base.abs() > 1.0, "File size log base must be > 1.0");
            let reverse = log_base < 0.0;
            Some(FileSizeClassifier::new(
                log_base.abs(),
                build_args.file_size.file_size_offset,
                reverse,
            ))
        } else {
            None
        };

        let dir_size_classifier = if let Some(log_base) = build_args.dir_size.dir_size_bias {
            assert!(
                log_base.abs() > 1.0,
                "Directory size log base must be > 1.0"
            );
            let reverse = log_base < 0.0;
            Some(DirSizeClassifier::new(
                log_base.abs(),
                build_args.dir_size.dir_size_offset,
                reverse,
            ))
        } else {
            None
        };

        let file_age_classifier = if let Some(log_base) = build_args.file_age.file_age_bias {
            assert!(log_base.abs() > 1.0, "File age log base must be > 1.0");
            let reverse = log_base < 0.0;
            Some(FileAgeClassifier::new(
                log_base.abs(),
                build_args.file_age.file_age_offset,
                reverse,
            ))
        } else {
            None
        };

        Self {
            build_args,
            entries: Vec::new(),
            tokenizer: None,
            frequent_ngrams: None,
            file_size_classifier,
            dir_size_classifier,
            file_age_classifier,
            naive_bayes: NaiveBayesClassifier::new(false),
            visualizer,
            playlist,
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

    fn collect_unclassified_files(&mut self) {
        // Create set of already classified paths (convert relative paths to absolute)
        let mut classified_paths = HashSet::new();
        let playlist_dir = self.playlist.root();

        // Add all entries (both positive and negative) to the classified set
        for entry in self.playlist.entries() {
            let abs_path = playlist_dir.join(entry.path());
            let canon = abs_path.canonicalize().unwrap_or_else(|e| {
                warn!("Unable to canonicalize {:?}, {:?}", abs_path, e);
                abs_path
            });
            classified_paths.insert(canon);
        }

        let walk = Walk::new(self.build_args.video_exts.iter().map(String::as_ref));
        for dir in &self.build_args.dirs {
            if let Err(e) = walk.walk_dir(dir) {
                error!("Error walking directory {}: {}", dir.display(), e);
                continue;
            }
        }

        let classifiers_len = self.get_classifiers().len();

        let file_receiver = walk.into_rx();
        while let Ok(file) = file_receiver.recv() {
            debug!("{:?}", file);

            let file_path = file.dir.join(&file.file_name);

            // Skip if already classified
            if classified_paths.contains(&file_path) {
                debug!("Skipping already classified file: {:?}", file_path);
                continue;
            }

            let normalized_path = normalize::normalize(&file_path);

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

        info!("Collected {} unclassified entries", self.entries.len());
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
        paths.extend(
            self.playlist
                .entries()
                .iter()
                .map(|e| normalize::normalize(e.path())),
        );

        // Create tokenizer from all paths
        self.tokenizer = Some(tokenize::PairTokenizer::new(
            paths.iter().map(String::as_str),
        ));
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
        paths.extend(
            self.playlist
                .entries()
                .iter()
                .map(|e| normalize::normalize(e.path())),
        );

        // Use the new function in ngrams.rs to count and filter
        self.frequent_ngrams = Some(Ngrams::count_and_filter_from_paths(
            &paths,
            tokenizer,
            self.build_args.windows,
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
                self.build_args.windows,
                self.frequent_ngrams.as_ref(),
                None, // No debug info needed here
            );
            entry.ngrams = Some(ngrams);
        }
    }

    // Trains the NaiveBayesClassifier using the tokenized and ngramized playlist entries.
    fn train_naive_bayes_classifier(&mut self) {
        let tokenizer = self.tokenizer.as_ref().unwrap();

        // Train naive bayes classifier on playlist entries
        let mut temp_ngrams = Ngrams::default();
        let playlist_dir = self.playlist.path().parent().unwrap_or(Path::new(""));

        // Process all examples in a single loop
        for entry in self.playlist.entries().iter() {
            let path = entry.path();
            let abs_path = playlist_dir.join(path);
            let canon = normalize::canonicalize_path(&abs_path);
            let normalized_path = normalize::normalize(&canon);
            let tokens = tokenizer.tokenize(&normalized_path);
            // Original code used None for allowed ngrams during training
            temp_ngrams.windows(&tokens, self.build_args.windows, None, None);

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
        let file_name = Some(entry.file.file_name.to_string_lossy().to_string());

        // Start VLC and get classification
        let vlc = vlc::VLCProcessHandle::new(&self.build_args.vlc, &path, file_name)
            .expect("failed to start vlc process");

        // Wait for VLC to start and verify filename
        if let Err(e) = vlc.wait_for_status() {
            error!("VLC startup error {:?}", e);
            return None;
        }

        match vlc.get_classification() {
            Ok(classification) => {
                if matches!(classification, vlc::Classification::Skipped) {
                    error!("Classification skipped for {:?}", path);
                    None
                } else {
                    Some(classification)
                }
            }
            Err(e) => {
                error!("Classification error: {:?}", e);
                None
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
                self.build_args.windows,
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

        // Update dir size classifier
        if let Some(ref mut dir_classifier) = self.dir_size_classifier {
            dir_classifier.remove_entry(&entry);
        }

        match classification {
            vlc::Classification::Positive => {
                self.playlist.add_positive(&path)?;
                self.naive_bayes
                    .train_positive(entry.ngrams.as_ref().unwrap());
                info!("{:?} (POSITIVE)", path);
            }
            vlc::Classification::Negative => {
                self.playlist.add_negative(&path)?;
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

            let num_to_process =
                std::cmp::min(self.entries.len(), std::cmp::max(self.build_args.top_n, 1));
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
    fn run(&mut self) -> Result<(), Error> {
        self.set_threads_to_min_priority();

        // 1. Reading files (assuming this happens during walk_dir and collect_unclassified_files)
        time_it!("File Reading and Collection", {
            self.collect_unclassified_files();
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
        if self.build_args.dry_run {
            info!("Dry run enabled. Skipping classification loop.");
            return Ok(());
        }

        self.classification_loop()?;

        Ok(())
    }
}

fn move_playlist(original_path: &Path, new_path: &Path) -> Result<(), Error> {
    // Load the original playlist
    let original_playlist = M3uPlaylist::open(original_path)?;
    let original_root = original_playlist.root();

    // Create a new playlist at the target location
    let mut new_playlist = M3uPlaylist::open(new_path)?;

    info!(
        "Moving playlist from {} to {}",
        original_playlist.path().display(),
        new_playlist.path().display()
    );

    // Process all entries in original order
    for entry in original_playlist.entries() {
        let abs_path = &original_root.join(entry.path());
        let canon = normalize::canonicalize_path(abs_path);

        // Add to new playlist based on entry type
        match entry {
            PlaylistEntry::Positive(_) => {
                new_playlist.add_positive(&canon)?;
                debug!("Moved positive entry: {}", canon.display());
            }
            PlaylistEntry::Negative(_) => {
                new_playlist.add_negative(&canon)?;
                debug!("Moved negative entry: {}", canon.display());
            }
        }
    }

    println!(
        "Successfully moved playlist from {} to {}",
        original_playlist.path().display(),
        new_playlist.path().display()
    );

    Ok(())
}

enum ListFilter {
    Positive,
    Negative,
}

fn list_entries(playlist_path: &Path, filter: ListFilter) -> Result<(), Error> {
    let playlist = M3uPlaylist::open(playlist_path)?;
    let root = playlist.path().parent().unwrap_or(Path::new(""));
    for entry in playlist.entries() {
        match (&filter, entry) {
            (ListFilter::Positive, PlaylistEntry::Positive(_)) => {
                let path = entry.path();
                let canon = normalize::canonicalize_path(&root.join(path));
                println!("{}", canon.display());
            }
            (ListFilter::Negative, PlaylistEntry::Negative(_)) => {
                let path = entry.path();
                let canon = normalize::canonicalize_path(&root.join(path));
                println!("{}", canon.display());
            }
            _ => {}
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    if std::env::var("RUST_LOG").is_err() {
        unsafe { std::env::set_var("RUST_LOG", &args.log_level) };
    }
    env_logger::init();

    match args.command {
        Command::Build(ref build_args) => {
            let playlist = M3uPlaylist::open(&build_args.playlist)?;
            let mut app = Build::new(build_args.clone(), playlist);
            app.run()?;
        }
        Command::ListPositive(list_args) => {
            list_entries(&list_args.playlist, ListFilter::Positive)?;
        }
        Command::ListNegative(list_args) => {
            list_entries(&list_args.playlist, ListFilter::Negative)?;
        }
        Command::Move(move_args) => {
            move_playlist(&move_args.original, &move_args.new)?;
        }
    }
    Ok(())
}
