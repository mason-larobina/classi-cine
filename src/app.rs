use crate::Error;
use crate::cache::{Cache, MediaFeatures, entry_hash};
use crate::classifier::{
    Classifier, DirSizeClassifier, FileAgeClassifier, FileSizeClassifier, NaiveBayesClassifier,
};
use crate::features::{self, FeatureConfig};
use crate::logging::time_it;
use crate::ngrams::{Ngram, Ngrams};
use crate::normalize;
use crate::path::{AbsPath, PathDisplayContext};
use crate::playlist::{M3uPlaylist, Playlist};
use crate::tokenize::PairTokenizer;
use crate::tokens::{Token, Tokens};
use crate::vlc;
use crate::walk::Walk;
use crate::{BuildArgs, ScoreArgs};

use crossterm::{
    cursor::Show,
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{
        self, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use log::*;
use rand::RngExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use thread_priority::*;

/// RAII guard owning the TUI terminal. Setting it up enables raw mode and the
/// alternate screen and suppresses stderr logging; dropping it (on normal
/// return, `?`, or unwinding from a panic) restores the terminal.
struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self, Error> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        // The TUI now owns the terminal; suppress stderr logging until restored.
        crate::logging::set_tui_active(true);
        let backend = CrosstermBackend::new(stdout);
        Ok(Self {
            terminal: Terminal::new(backend)?,
        })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Restore the terminal to its normal state: leave raw mode and the alternate
/// screen, re-show the cursor, and resume stderr logging. Errors are ignored
/// because this runs on cleanup and panic paths where they can't be propagated,
/// and the operation is idempotent so it's safe to call more than once.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        Show
    );
    crate::logging::set_tui_active(false);
}

/// Install a panic hook that restores the terminal before delegating to the
/// previous hook. Without this the default panic message would be printed into
/// the alternate screen and lost when the terminal is torn down.
pub fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original(info);
    }));
}

/// Format a byte count as a compact human-readable string (binary units).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{}{}", bytes, UNITS[unit]); // whole bytes, no decimal
    }
    // Keep ~3 significant figures so precision is consistent across magnitudes
    // (e.g. "700M", "50.5M", "1.10G") instead of always showing one decimal.
    let decimals = |s: f64| {
        if s >= 100.0 {
            0
        } else if s >= 10.0 {
            1
        } else {
            2
        }
    };
    // If rounding to the chosen precision would reach 1024, promote a unit so
    // we show e.g. "1.00G" rather than "1024M".
    let mut d = decimals(size);
    let factor = 10f64.powi(d as i32);
    if (size * factor).round() / factor >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
        d = decimals(size);
    }
    format!("{:.*}{}", d, size, UNITS[unit])
}

/// Format a duration in seconds as a compact `HhMmSs` / `MmSs` / `Ss`
/// string for the media-features TUI panel.
fn format_secs(secs: f64) -> String {
    if !secs.is_finite() || secs <= 0.0 {
        return "-".to_string();
    }
    let total = secs.round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}h{}m{}s", h, m, s)
    } else if m > 0 {
        format!("{}m{}s", m, s)
    } else {
        format!("{}s", s)
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: AbsPath,
    // Cached arc parent directory for efficient use
    pub parent_dir: Arc<PathBuf>,
    pub size: u64,
    pub created: SystemTime,
    pub normalized_path: String,
    pub tokens: Option<Tokens>,
    pub ngrams: Option<Ngrams>,
    pub scores: Box<[f64]>, // One score per classifier
    /// Extracted ffprobe features for this file, plumbed through from the
    /// ffprobe cache. Associated (required) on every builder entry and
    /// persisted with the classification result. Defaults to all-zero when a
    /// file was not present in the cache (e.g. its probe failed); the field
    /// is still required so the classifier can rely on its presence.
    pub features: MediaFeatures,
}

#[derive(Serialize)]
struct ScoreEntry {
    score: f64,
    filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

#[derive(Serialize)]
struct DirScoreEntry {
    average_score: f64,
    entry_count: usize,
    total_size: u64,
    directory: String,
}

pub struct App {
    common_args: crate::CommonArgs,
    build_args: Option<crate::BuildArgs>,
    score_args: Option<crate::ScoreArgs>,
    entries: Vec<Entry>,
    tokenizer: Option<PairTokenizer>,
    frequent_ngrams: Option<ahash::AHashSet<Ngram>>,
    file_size_classifier: Option<FileSizeClassifier>,
    dir_size_classifier: Option<DirSizeClassifier>,
    file_age_classifier: Option<FileAgeClassifier>,
    naive_bayes: NaiveBayesClassifier,
    playlist: M3uPlaylist,
    vlc_controller: Option<vlc::VlcController>,
    // Every file seen by the walk (before classification filtering), retained
    // as the live-key set for the ffprobe cache populate pass.
    collected_files: Vec<crate::walk::File>,
    // TUI state
    list_state: ListState,
    currently_playing: Option<usize>,
    should_quit: bool,
    terminal_height: u16,
}

impl App {
    pub fn new(build_args: BuildArgs, playlist: M3uPlaylist) -> Self {
        Self::new_common(
            build_args.common.clone(),
            Some(build_args.clone()),
            None, // no score args for build command
            playlist,
        )
    }

    pub fn new_for_scoring(score_args: ScoreArgs, playlist: M3uPlaylist) -> Self {
        Self::new_common(
            score_args.common.clone(),
            None,
            Some(score_args.clone()),
            playlist,
        )
    }

    fn new_common(
        common_args: crate::CommonArgs,
        build_args: Option<crate::BuildArgs>,
        score_args: Option<crate::ScoreArgs>,
        playlist: M3uPlaylist,
    ) -> Self {
        info!("{:#?}", common_args);

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

        let vlc_controller = build_args
            .as_ref()
            .map(|args| vlc::VlcController::new(args.vlc.clone()));

        // Get initial terminal height
        let terminal_height = terminal::size().map(|(_, h)| h).unwrap_or(20);

        Self {
            common_args,
            build_args,
            score_args,
            entries: Vec::new(),
            tokenizer: None,
            frequent_ngrams: None,
            file_size_classifier,
            dir_size_classifier,
            file_age_classifier,
            naive_bayes: NaiveBayesClassifier::new(false),
            playlist,
            vlc_controller,
            collected_files: Vec::new(),
            list_state: ListState::default(),
            currently_playing: None,
            should_quit: false,
            terminal_height,
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

    fn collect_files(&mut self) {
        // Create set of already classified paths (convert relative paths to absolute)
        let mut classified_paths = HashSet::new();

        // Cache for deduplicating Arc<PathBuf> parent directories
        let mut parent_dir_cache: HashMap<PathBuf, Arc<PathBuf>> = HashMap::new();

        let include_classified = self
            .score_args
            .as_ref()
            .map(|args| args.include_classified)
            .unwrap_or(false);
        if !include_classified {
            // Add all entries (both positive and negative) to the classified set
            for entry in self.playlist.entries() {
                let abs_path = entry.abs_path(self.playlist.root()).to_path_buf();
                classified_paths.insert(abs_path);
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

            // Retain every walked file for the ffprobe cache populate pass
            // (the live-key set is all collected files, not just unclassified
            // candidates), regardless of the classification filter below.
            self.collected_files.push(file.clone());

            let abs_file_path = &file.path;
            let normalized_file_path = file.path.to_path_buf();

            // Skip if already classified (only when include_classified is false)
            if !include_classified && classified_paths.contains(&normalized_file_path) {
                debug!("Skipping already classified file: {:?}", abs_file_path);
                continue;
            }

            let context = PathDisplayContext::RelativeTo(self.playlist.root().to_path_buf());
            let path_to_normalize = file.path.to_string(&context);
            let normalized_path = normalize::normalize(&path_to_normalize);

            let mut scores = vec![0.0; classifiers_len];
            scores.shrink_to_fit();

            // Initialize entry with scores array sized for all classifiers plus naive bayes
            // Use deduplication to share Arc<PathBuf> for files in the same directory
            let parent_path = file.path.parent().unwrap().to_path_buf();
            let parent_dir = if let Some(existing_arc) = parent_dir_cache.get(&parent_path) {
                Arc::clone(existing_arc)
            } else {
                let new_arc = Arc::new(parent_path.clone());
                parent_dir_cache.insert(parent_path, Arc::clone(&new_arc));
                new_arc
            };

            let entry = Entry {
                path: file.path,
                size: file.size,
                created: file.created,
                normalized_path,
                parent_dir,
                tokens: None,
                ngrams: None,
                scores: scores.into_boxed_slice(),
                features: MediaFeatures::default(),
            };

            // Update dir size classifier if present
            if let Some(ref mut dir_classifier) = self.dir_size_classifier {
                dir_classifier.add_entry(&entry);
            }

            self.entries.push(entry);
        }

        if include_classified {
            info!(
                "Collected {} entries (including classified)",
                self.entries.len()
            );
        } else {
            info!("Collected {} unclassified entries", self.entries.len());
        }
    }

    /// Run the ffprobe feature cache populate pass (load → compact →
    /// write-survivors → delete → probe+write-missing) over every file seen by
    /// the walk. See `docs/ffprobe-cache.md`. The cache lives under the XDG
    /// cache directory and is rewritten as a fresh shard generation every
    /// startup. Never aborts the run: a missing/unusable cache dir degrades to
    /// a full re-probe, and per-file probe failures leave the file uncached
    /// (retried next run).
    ///
    /// Returns the `entry_hash → MediaFeatures` lookup for every entry now on
    /// disk; [`attach_features`](Self::attach_features) uses it to populate
    /// each builder entry's `features`.
    fn populate_cache(&self) -> HashMap<String, MediaFeatures> {
        let Some(cache) = Cache::with_default_dir(self.common_args.cache_ttl_days) else {
            warn!("ffprobe cache: cannot resolve cache directory; skipping");
            return HashMap::new();
        };
        let probe = crate::ffprobe::FfprobeProbe::new();
        cache.populate(&self.collected_files, &probe)
    }

    /// Attach the ffprobe-extracted features to every builder entry by looking
    /// each file up in the cache map by its `(path, mtime, size)` key. Files
    /// absent from the map (e.g. their probe failed) keep the all-zero default
    /// — the field stays present and required, just unpopulated.
    fn attach_features(&mut self, features_map: &HashMap<String, MediaFeatures>) {
        for entry in &mut self.entries {
            let key = entry_hash(&entry.path, entry.created, entry.size);
            if let Some(features) = features_map.get(&key) {
                entry.features = features.clone();
            }
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
        let context = PathDisplayContext::RelativeTo(self.playlist.root().to_path_buf());
        paths.extend(self.playlist.entries().iter().map(|e| {
            let path_to_normalize = e.abs_path(self.playlist.root()).to_string(&context);
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
    //
    // Feature ngrams are appended into the **same** per-entry `Ngrams` vec as
    // the path ngrams and consumed by the single existing
    // `NaiveBayesClassifier` (no second instance, no extra score column). See
    // `docs/media-features-classifier.md` *Pipeline*.
    fn generate_ngrams(&mut self) {
        let windows = self.common_args.windows;
        let combinations = self.common_args.combinations;
        let cfg = FeatureConfig::from_common(&self.common_args);
        let features_combinations = cfg.combinations;

        let context = PathDisplayContext::RelativeTo(self.playlist.root().to_path_buf());

        // Collect all paths for ngram counting (candidates + playlist).
        let mut paths: Vec<String> = self
            .entries
            .iter()
            .map(|e| e.normalized_path.to_string())
            .collect();
        paths.extend(self.playlist.entries().iter().map(|e| {
            let path_to_normalize = e.abs_path(self.playlist.root()).to_string(&context);
            normalize::normalize(&path_to_normalize)
        }));

        // Build feature Tokens per entry (candidates ++ playlist), minting each
        // token string into the **same `TokenMap` the path tokenizer owns**.
        // The vec is aligned 1:1 with `paths` so the frequent-set counting pass
        // can fold feature combinations into the same `AHashMap<Ngram, u8>`.
        let n_candidates = self.entries.len();
        let all_feature_tokens: Vec<Tokens> = {
            let tokenizer = self.tokenizer.as_mut().unwrap();
            let map = tokenizer.token_map_mut();
            // Build in two explicit loops (rather than a chained iterator) so
            // the single `&mut TokenMap` is shared sequentially across both
            // candidate and playlist feature minting without two closures
            // simultaneously capturing it.
            let mut v = Vec::with_capacity(n_candidates + self.playlist.entries().len());
            for e in &self.entries {
                v.push(features::build_feature_tokens(&e.features, map, &cfg));
            }
            for e in self.playlist.entries() {
                v.push(features::build_feature_tokens(&e.features, map, &cfg));
            }
            v
        };

        let tokenizer = self.tokenizer.as_ref().unwrap();
        let last_special = tokenizer.token_map().last_special();

        // Count frequent ngrams over the merged space (path + feature); store
        // `entry.ngrams` (paths + features, windows+combinations, deduped
        // together).
        self.frequent_ngrams = Some(Ngrams::count_and_filter_from_paths(
            &paths,
            &all_feature_tokens,
            tokenizer,
            windows,
            combinations,
            features_combinations,
        ));

        info!("total paths {:?}", paths.len());
        info!(
            "frequent ngrams {:?}",
            self.frequent_ngrams.as_ref().unwrap().len()
        );

        // Final pass to store tokens and frequent ngrams for candidates only.
        // Feature combinations are appended into the same `Ngrams` vec that
        // already holds the path windows + path combinations; the
        // sort-and-dedup in `combinations` collapses any duplicates across the
        // three generation passes.
        let frequent = self.frequent_ngrams.as_ref();
        let candidate_feature_tokens = &all_feature_tokens[..n_candidates];
        for (entry, ft) in self.entries.iter_mut().zip(candidate_feature_tokens.iter()) {
            // Tokenize the path and store the tokens
            entry.tokens = Some(tokenizer.tokenize(&entry.normalized_path));

            let mut ngrams = Ngrams::default();
            // Generate ngrams for the entry using the frequent filter
            ngrams.windows(
                entry.tokens.as_ref().unwrap(),
                windows,
                frequent,
                None, // No debug info needed here
            );
            ngrams.combinations(
                entry.tokens.as_ref().unwrap(),
                combinations,
                last_special,
                frequent,
                None,
            );
            if features_combinations > 0 {
                ngrams.combinations(ft, features_combinations, last_special, frequent, None);
            }
            entry.ngrams = Some(ngrams);
        }
    }

    // Trains the NaiveBayesClassifier using the tokenized and ngramized playlist entries.
    //
    // Feature combinations are appended into the same per-entry `Ngrams` as the
    // path windows/combinations, so the single `naive_bayes` classifier trains
    // on the merged vec (path + feature ngrams) exactly as it scores it.
    fn train_naive_bayes_classifier(&mut self) {
        let windows = self.common_args.windows;
        let combinations = self.common_args.combinations;
        let cfg = FeatureConfig::from_common(&self.common_args);
        let features_combinations = cfg.combinations;

        let context = PathDisplayContext::RelativeTo(self.playlist.root().to_path_buf());

        // Collect playlist training data up front so the tokenizer can be
        // mutably borrowed (to mint feature tokens) without holding a borrow
        // of `self.playlist` / `self.naive_bayes` across the minting loop.
        let training: Vec<(String, MediaFeatures, bool)> = self
            .playlist
            .entries()
            .iter()
            .map(|e| {
                let path_to_normalize = e.abs_path(self.playlist.root()).to_string(&context);
                let normalized_path = normalize::normalize(&path_to_normalize);
                (normalized_path, e.features.clone(), e.is_positive())
            })
            .collect();

        // Build the merged `Ngrams` per playlist entry. Training uses no
        // frequent-set filter (same as the path-only path), so all feature
        // combinations are trained — rare ones enter the vocabulary and are
        // then Laplace-smoothed at score time, mirroring how rare path ngrams
        // behave.
        let built: Vec<(Ngrams, bool)> = {
            let tokenizer = self.tokenizer.as_mut().unwrap();
            let last_special = tokenizer.token_map().last_special();
            // Tokenize all paths first (immutable borrow), producing owned
            // `Tokens`, so the mutable `TokenMap` borrow for feature minting
            // does not overlap the `tokenize` calls.
            let tokenized: Vec<(Tokens, MediaFeatures, bool)> = training
                .iter()
                .map(|(path, features, is_positive)| {
                    (tokenizer.tokenize(path), features.clone(), *is_positive)
                })
                .collect();
            let map = tokenizer.token_map_mut();
            tokenized
                .into_iter()
                .map(|(tokens, features, is_positive)| {
                    let mut ngrams = Ngrams::default();
                    ngrams.windows(&tokens, windows, None, None);
                    ngrams.combinations(&tokens, combinations, last_special, None, None);
                    if features_combinations > 0 {
                        let ft = features::build_feature_tokens(&features, map, &cfg);
                        ngrams.combinations(&ft, features_combinations, last_special, None, None);
                    }
                    (ngrams, is_positive)
                })
                .collect()
        };

        for (ngrams, is_positive) in built {
            if is_positive {
                self.naive_bayes.train_positive(&ngrams);
            } else {
                self.naive_bayes.train_negative(&ngrams);
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
            ) && (max - min).abs() > f64::EPSILON
            {
                for (entry, &raw_score) in temp_entries.iter_mut().zip(&col_scores) {
                    entry.scores[col] = (raw_score - min) / (max - min);
                }
            }
        }

        // Sort entries by total score descending (highest scores first)
        temp_entries.sort_by(|a, b| {
            let a_sum = a.scores.iter().sum::<f64>();
            let b_sum = b.scores.iter().sum::<f64>();
            b_sum.partial_cmp(&a_sum).expect("Invalid score comparison")
        });

        // Swap back the processed entries
        std::mem::swap(&mut self.entries, &mut temp_entries);
    }

    // Updates classifiers and playlist with the classification result
    fn process_classification_result(
        &mut self,
        entry: Entry,
        classification: vlc::Classification,
    ) -> Result<(), Error> {
        let abs_path = &entry.path;

        // Update dir size classifier
        if let Some(ref mut dir_classifier) = self.dir_size_classifier {
            dir_classifier.remove_entry(&entry);
        }

        match classification {
            vlc::Classification::Positive => {
                self.playlist.add_positive(abs_path, &entry.features)?;
                self.naive_bayes
                    .train_positive(entry.ngrams.as_ref().unwrap());
            }
            vlc::Classification::Negative => {
                self.playlist.add_negative(abs_path, &entry.features)?;
                self.naive_bayes
                    .train_negative(entry.ngrams.as_ref().unwrap());
            }
            vlc::Classification::Skipped => unreachable!(), // Handled in poll_classification
        }

        Ok(())
    }

    fn init(&mut self) {
        self.set_threads_to_min_priority();

        time_it!("File Reading and Collection", {
            self.collect_files();
        });

        // Run the ffprobe cache populate pass before tokenization/training:
        // probing is eager (the tokenizer/classifiers train over the full
        // corpus) and the cache is fully persisted to disk before control
        // returns here. Never aborts the run — degrades to an empty cache on
        // any failure. The returned features map is attached to each builder
        // entry so the features travel with the entry and are persisted with
        // its classification result.
        let features_map = time_it!("ffprobe cache populate", { self.populate_cache() });
        time_it!("Attach media features", {
            self.attach_features(&features_map);
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
    }

    fn draw_tui(&mut self, f: &mut Frame) {
        // Create horizontal split: left for file list, right for debug info
        let main_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .margin(1)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(f.area());

        // Draw file list on the left
        self.draw_file_list(f, main_chunks[0]);

        // Draw debug panel on the right
        self.draw_debug_panel(f, main_chunks[1]);
    }

    fn tui_len(&self) -> usize {
        self.entries.len()
    }

    fn draw_file_list(&mut self, f: &mut Frame, area: Rect) {
        let context = PathDisplayContext::build_context(self.playlist.root());

        // Only build ListItems for a window of entries around the selection
        // cursor rather than the whole list, which may contain 100k+ entries.
        // ratatui then positions the viewport within this window; we translate
        // its window-local offset back to a global offset afterwards so the
        // list_state remains the global source of truth.
        let sel = self.list_state.selected().unwrap_or(0);
        let view_h = area.height.saturating_sub(2) as usize; // minus borders
        let buffer = view_h.max(1); // headroom each side for smooth scrolling
        let start = sel.saturating_sub(buffer);
        let end = (sel + buffer + view_h.max(1)).min(self.entries.len());

        let items: Vec<ListItem> = self.entries[start..end]
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let global_idx = start + i;
                let filename = self.playlist.display_path(&entry.path, &context);
                let total_score: f64 = entry.scores.iter().sum();
                let display_text = format!(
                    "{:.3} {:>6} {}",
                    total_score,
                    human_size(entry.size),
                    filename
                );

                let style = if Some(global_idx) == self.currently_playing {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let content = Line::from(Span::styled(display_text, style));
                ListItem::new(content)
            })
            .collect();

        let title = format!(
            "File List ({}) (↑/↓: navigate, Enter: play, Esc/q: quit)",
            self.entries.len()
        );
        let list = List::new(items)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("► ");

        // Render with a local state in window-local coordinates, then map the
        // offset ratatui computed back into a global offset.
        let mut local = ListState::default();
        local.select(Some(sel - start));
        *local.offset_mut() = self.list_state.offset().saturating_sub(start);
        f.render_stateful_widget(list, area, &mut local);
        *self.list_state.offset_mut() = start + local.offset();
    }

    fn draw_debug_panel(&mut self, f: &mut Frame, area: Rect) {
        // Get the currently selected entry
        let selected_entry = if let Some(selected_idx) = self.list_state.selected() {
            if selected_idx < self.entries.len() {
                Some(self.entries[selected_idx].clone())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(entry) = selected_entry {
            // Split debug panel into sections.
            let debug_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(6), // Path info (full path + tokenized)
                        Constraint::Length(8), // Media features (raw + tokens)
                        Constraint::Length(6), // Classifier scores
                        Constraint::Min(0),    // N-grams
                    ]
                    .as_ref(),
                )
                .split(area);

            self.draw_path_info(f, debug_chunks[0], &entry);
            self.draw_media_features(f, debug_chunks[1], &entry);
            self.draw_classifier_scores(f, debug_chunks[2], &entry);
            self.draw_ngrams(f, debug_chunks[3], &entry);
        } else {
            // No selection - show empty panel
            let block = Block::default().title("Debug Info").borders(Borders::ALL);
            f.render_widget(block, area);
        }
    }

    fn draw_path_info(&mut self, f: &mut Frame, area: Rect, entry: &Entry) {
        let mut lines = Vec::new();

        // Add full path
        let full_path = entry.path.to_string_lossy().to_string();
        lines.push(Line::from(Span::styled(
            format!("Path: {:?}", full_path),
            Style::default(),
        )));

        // Add tokenized path
        let tokenized_text = if let Some(ref tokens) = entry.tokens {
            if let Some(tokenizer) = &self.tokenizer {
                let token_map = tokenizer.token_map();
                let token_strs: Vec<&str> = tokens
                    .as_slice()
                    .iter()
                    .map(|t| token_map.get_str(*t).unwrap_or("<unknown>"))
                    .collect();
                format!("Tokens: {:?}", token_strs)
            } else {
                "Tokens: No tokenizer available".to_string()
            }
        } else {
            "Tokens: No tokens available".to_string()
        };

        lines.push(Line::from(Span::styled(tokenized_text, Style::default())));

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Path Information")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    /// Render the selected entry's extracted ffprobe [`MediaFeatures`] as both
    /// human-readable raw values and the feature-token strings the classifier
    /// actually consumes (categorical singletons + bucketed neighbor singletons
    /// for the continuous fields). The tokens are computed via the pure
    /// [`features::feature_token_strings`] so the render path never mutates the
    /// shared `TokenMap`.
    fn draw_media_features(&self, f: &mut Frame, area: Rect, entry: &Entry) {
        let feat = &entry.features;
        let cfg = FeatureConfig::from_common(&self.common_args);

        // Compact human-readable summary of the raw probe values.
        let duration = if feat.duration_secs > 0.0 {
            format_secs(feat.duration_secs)
        } else {
            "-".to_string()
        };
        let bitrate = if feat.duration_secs > 0.0 {
            human_size(((feat.file_size as f64) * 8.0 / feat.duration_secs) as u64) + "bps"
        } else {
            "-".to_string()
        };
        let resolution = if feat.width > 0 && feat.height > 0 {
            format!("{}x{}", feat.width, feat.height)
        } else {
            "-".to_string()
        };
        let fps = match feat.fps {
            Some(v) => format!("{:.3}", v),
            None => "-".to_string(),
        };
        let audio = if feat.audio_codec.is_empty() {
            if features::has_probe_data(feat) {
                "none"
            } else {
                "-"
            }
        } else {
            feat.audio_codec.as_str()
        };
        let video = if feat.video_codec.is_empty() {
            "-"
        } else {
            feat.video_codec.as_str()
        };
        let raw = format!(
            "video:{} audio:{} res:{} dur:{} size:{} br:{} fps:{}",
            video,
            audio,
            resolution,
            duration,
            human_size(feat.file_size),
            bitrate,
            fps,
        );

        let token_strings = features::feature_token_strings(feat, &cfg);
        let tokens_line = if token_strings.is_empty() {
            "(none — probe failed or no signal)".to_string()
        } else {
            token_strings.join("  ")
        };

        let lines = vec![
            Line::from(Span::styled(format!("raw:  {}", raw), Style::default())),
            Line::from(Span::styled(
                format!("toks: {}", tokens_line),
                Style::default().fg(Color::Cyan),
            )),
        ];

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .title("Media Features")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }

    fn draw_classifier_scores(&mut self, f: &mut Frame, area: Rect, entry: &Entry) {
        let classifiers = self.get_classifiers();

        // Create the block with border
        let block = Block::default()
            .title("Classifier Scores")
            .borders(Borders::ALL);

        // Get the inner area after accounting for the border
        let score_area = block.inner(area);

        // Render the block with border
        f.render_widget(block, area);

        if score_area.height >= classifiers.len() as u16 {
            let bar_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Length(1); classifiers.len()])
                .split(score_area);

            for (i, classifier) in classifiers.iter().enumerate() {
                if i < bar_chunks.len() && i < entry.scores.len() {
                    let score = entry.scores[i];
                    let normalized_score = score.clamp(0.0, 1.0); // Scores should already be normalized 0-1

                    // Split each row into name and gauge areas
                    let row_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Length(16), Constraint::Min(0)])
                        .split(bar_chunks[i]);

                    // Render classifier name
                    let name_label = Paragraph::new(classifier.name()).style(Style::default());
                    f.render_widget(name_label, row_chunks[0]);

                    // Render gauge with score as label
                    let color = if score > 0.0 {
                        Color::Green
                    } else {
                        Color::Red
                    };
                    let gauge = Gauge::default()
                        .block(Block::default())
                        .gauge_style(Style::default().fg(color))
                        .ratio(normalized_score)
                        .label(format!("{:.3}", score));

                    f.render_widget(gauge, row_chunks[1]);
                }
            }
        }
    }

    fn draw_ngrams(&mut self, f: &mut Frame, area: Rect, entry: &Entry) {
        if let Some(ref tokens) = entry.tokens {
            if let Some(tokenizer) = &self.tokenizer {
                let token_map = tokenizer.token_map();

                // Regenerate ngram tokens (same method as existing debug code)
                let mut ngram_tokens: Vec<Vec<Token>> = Vec::new();
                {
                    let mut tmp_ngrams = Ngrams::default();
                    tmp_ngrams.windows(
                        tokens,
                        self.common_args.windows,
                        self.frequent_ngrams.as_ref(),
                        Some(&mut ngram_tokens),
                    );
                    tmp_ngrams.combinations(
                        tokens,
                        self.common_args.combinations,
                        token_map.last_special(),
                        self.frequent_ngrams.as_ref(),
                        Some(&mut ngram_tokens),
                    );
                    ngram_tokens.sort();
                    ngram_tokens.dedup();
                }

                // Get ngram scores
                let mut ngram_scores = Vec::new();
                for window in ngram_tokens.into_iter() {
                    let ngram = Ngram::new(&window);
                    let score = self.naive_bayes.ngram_score(ngram);
                    ngram_scores.push((window, score));
                }

                // Sort by absolute score (most influential first)
                ngram_scores.sort_by(|a, b| {
                    b.1.abs()
                        .partial_cmp(&a.1.abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                // Take top 50 for display
                ngram_scores.truncate(50);

                let ngram_lines: Vec<Line> = ngram_scores
                    .iter()
                    .map(|(tokens, score)| {
                        let token_strs: Vec<&str> = tokens
                            .iter()
                            .map(|t| token_map.get_str(*t).unwrap())
                            .collect();

                        let color = if *score > 0.0 {
                            Color::Green
                        } else {
                            Color::Red
                        };
                        let bar_length = (score.abs() * 10.0) as usize;
                        let bar = "█".repeat(bar_length.min(10));
                        let line_text = format!("{:6.3} {:10} {:?}", score, bar, token_strs);
                        Line::from(Span::styled(line_text, Style::default().fg(color)))
                    })
                    .collect();

                let paragraph = Paragraph::new(ngram_lines)
                    .block(
                        Block::default()
                            .title("Top N-grams (sorted by influence)")
                            .borders(Borders::ALL),
                    )
                    .wrap(Wrap { trim: false });
                f.render_widget(paragraph, area);
            } else {
                let paragraph = Paragraph::new("No tokenizer available")
                    .block(Block::default().title("N-grams").borders(Borders::ALL));
                f.render_widget(paragraph, area);
            }
        } else {
            let paragraph = Paragraph::new("No tokens available")
                .block(Block::default().title("N-grams").borders(Borders::ALL));
            f.render_widget(paragraph, area);
        }
    }

    fn handle_tui_events(&mut self) -> Result<bool, Error> {
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                self.should_quit = true;
                            }
                            KeyCode::Char('c')
                                if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                            {
                                self.should_quit = true;
                            }
                            KeyCode::Down => {
                                self.tui_next();
                            }
                            KeyCode::Up => {
                                self.tui_previous();
                            }
                            KeyCode::Enter => {
                                self.tui_select_current()?;
                            }
                            KeyCode::PageUp => {
                                self.tui_page_up();
                            }
                            KeyCode::PageDown => {
                                self.tui_page_down();
                            }
                            KeyCode::Home => {
                                self.tui_home();
                            }
                            KeyCode::End => {
                                self.tui_end();
                            }
                            _ => {}
                        }
                    }
                }
                Event::Resize(_, height) => {
                    self.terminal_height = height;
                }
                _ => {}
            }
        }
        Ok(self.should_quit)
    }

    fn tui_next(&mut self) {
        let len = self.tui_len();
        if len > 0 {
            let i = match self.list_state.selected() {
                Some(i) => {
                    if i >= len - 1 {
                        0
                    } else {
                        i + 1
                    }
                }
                None => 0,
            };
            self.list_state.select(Some(i));
        }
    }

    fn tui_previous(&mut self) {
        let len = self.tui_len();
        if len > 0 {
            let i = match self.list_state.selected() {
                Some(i) => {
                    if i == 0 {
                        len - 1
                    } else {
                        i - 1
                    }
                }
                None => 0,
            };
            self.list_state.select(Some(i));
        }
    }

    fn tui_page_up(&mut self) {
        if self.tui_len() > 0 {
            let page_size = std::cmp::max(1, self.terminal_height / 2) as usize;
            let i = match self.list_state.selected() {
                Some(i) => i.saturating_sub(page_size),
                None => 0,
            };
            self.list_state.select(Some(i));
        }
    }

    fn tui_page_down(&mut self) {
        let len = self.tui_len();
        if len > 0 {
            let page_size = std::cmp::max(1, self.terminal_height / 2) as usize;
            let i = match self.list_state.selected() {
                Some(i) => {
                    let new_pos = i + page_size;
                    if new_pos >= len { len - 1 } else { new_pos }
                }
                None => 0,
            };
            self.list_state.select(Some(i));
        }
    }

    fn tui_home(&mut self) {
        if self.tui_len() > 0 {
            self.list_state.select(Some(0));
        }
    }

    fn tui_end(&mut self) {
        let len = self.tui_len();
        if len > 0 {
            self.list_state.select(Some(len - 1));
        }
    }

    fn tui_select_current(&mut self) -> Result<(), Error> {
        if let Some(selected) = self.list_state.selected()
            && selected < self.tui_len()
        {
            let entry = &self.entries[selected];
            let file_name = entry
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string());

            if let Some(ref vlc_controller) = self.vlc_controller {
                vlc_controller.start_playback(&entry.path, file_name)?;
                self.currently_playing = Some(selected);
            }
        }
        Ok(())
    }

    fn tui_auto_select_next(&mut self) -> Result<(), Error> {
        // First, update classification scores and sort entries (same as classification_loop)
        //time_it!("Update classification scores", {
        self.calculate_scores_and_sort_entries();
        //});

        // Use the same selection logic as the original classification_loop
        let selected_entry_idx = if let Some(build_args) = &self.build_args {
            if self.entries.is_empty() {
                return Ok(());
            }
            if let Some(p) = build_args.selection_p {
                let mut rng = rand::rng();
                self.entries
                    .iter()
                    .position(|_| rng.random::<f64>() <= p)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            if self.entries.is_empty() {
                return Ok(());
            }
            self.entries.len() - 1
        };

        self.list_state.select(Some(selected_entry_idx));

        let entry = &self.entries[selected_entry_idx];
        let file_name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string());

        if let Some(ref vlc_controller) = self.vlc_controller {
            vlc_controller.start_playback(&entry.path, file_name)?;
            self.currently_playing = Some(selected_entry_idx);
        }

        Ok(())
    }

    fn tui_handle_classification(&mut self) -> Result<(), Error> {
        if let Some(playing_idx) = self.currently_playing
            && let Some(ref vlc_controller) = self.vlc_controller
        {
            match vlc_controller.try_recv_classification() {
                Ok(Some(classification)) => {
                    if playing_idx < self.entries.len() {
                        let entry = self.entries.remove(playing_idx);
                        self.process_classification_result(entry, classification)?;

                        self.currently_playing = None;

                        // Auto-select and play next entry using build command logic
                        if !self.entries.is_empty() {
                            self.tui_auto_select_next()?;
                        } else {
                            self.list_state.select(None);
                            self.should_quit = true;
                        }
                    }
                }
                Ok(None) => {
                    // No classification yet, continue
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn run_tui_build(&mut self) -> Result<(), Error> {
        // Auto-select and play first file using build command logic
        if !self.entries.is_empty() {
            self.tui_auto_select_next()?;
        }

        // The guard restores the terminal when dropped, on every exit path.
        let mut guard = TerminalGuard::new()?;

        let result = (|| -> Result<(), Error> {
            loop {
                guard.terminal.draw(|f| self.draw_tui(f))?;

                if self.handle_tui_events()? {
                    break;
                }

                self.tui_handle_classification()?;

                if self.entries.is_empty() {
                    break;
                }
            }
            Ok(())
        })();

        drop(guard);
        result
    }

    pub fn run_build(&mut self) -> Result<(), Error> {
        // Restore the terminal if a panic unwinds through the TUI loop, so the
        // panic message lands on a clean screen instead of the alternate one.
        install_panic_hook();

        self.init();

        // For now, use TUI mode for build
        self.run_tui_build()?;

        Ok(())
    }

    pub fn run_score(&mut self) -> Result<(), Error> {
        self.init();

        // Calculate scores and sort entries
        time_it!("Calculate scores", {
            self.calculate_scores_and_sort_entries();
        });

        // Display all files with their scores
        let score_args = self.score_args.as_ref().unwrap();

        let context = PathDisplayContext::score_list_context(score_args.absolute);

        if score_args.by_dir {
            // Aggregate by directory
            let mut dir_aggregates: HashMap<String, (f64, usize, u64)> = HashMap::new();

            for entry in &self.entries {
                let total_score: f64 = entry.scores.iter().sum();

                let parent_dir_abs = AbsPath::from_abs_path(&entry.parent_dir);
                let dir_path = parent_dir_abs.to_string(&context);

                let size = entry.size;

                let (total_score_sum, count, total_size) =
                    dir_aggregates.entry(dir_path).or_insert((0.0, 0, 0));
                *total_score_sum += total_score;
                *count += 1;
                *total_size += size;
            }

            // Convert to dir score entries
            let mut dir_score_entries: Vec<DirScoreEntry> = dir_aggregates
                .into_iter()
                .map(
                    |(directory, (total_score_sum, count, total_size))| DirScoreEntry {
                        average_score: total_score_sum / count as f64,
                        entry_count: count,
                        total_size,
                        directory,
                    },
                )
                .collect();

            // Apply reverse ordering if requested
            if score_args.reverse {
                dir_score_entries
                    .sort_by(|a, b| a.average_score.partial_cmp(&b.average_score).unwrap());
            } else {
                dir_score_entries
                    .sort_by(|a, b| b.average_score.partial_cmp(&a.average_score).unwrap());
            }

            if score_args.json {
                // JSON output
                let json_output =
                    serde_json::to_string_pretty(&dir_score_entries).map_err(Error::SerdeJson)?;
                println!("{}", json_output);
            } else {
                // Text output
                if !score_args.no_header {
                    println!("AVG_SCORE\tENTRIES\tTOTAL_SIZE\tDIRECTORY");
                }

                for entry in &dir_score_entries {
                    println!(
                        "{:.3}\t{}\t{}\t{}",
                        entry.average_score, entry.entry_count, entry.total_size, entry.directory
                    );
                }
            }
        } else {
            // Collect score entries
            let mut score_entries: Vec<ScoreEntry> = Vec::new();

            for entry in &self.entries {
                let total_score: f64 = entry.scores.iter().sum();

                let size = if score_args.include_size {
                    Some(entry.size)
                } else {
                    None
                };

                let display_path = entry.path.to_string(&context);

                score_entries.push(ScoreEntry {
                    score: total_score,
                    filename: display_path,
                    size,
                });
            }

            // Apply reverse ordering if requested
            // Default is highest scores first (.rev() in original), so reverse=true means lowest first
            if score_args.reverse {
                // Keep current order (lowest scores first)
            } else {
                // Default behavior: highest scores first
                score_entries.reverse();
            }

            if score_args.json {
                // JSON output
                let json_output =
                    serde_json::to_string_pretty(&score_entries).map_err(Error::SerdeJson)?;
                println!("{}", json_output);
            } else {
                // Text output
                if !score_args.no_header {
                    if score_args.include_size {
                        println!("SCORE\tSIZE\tFILENAME");
                    } else {
                        println!("SCORE\tFILENAME");
                    }
                }

                for entry in &score_entries {
                    if let Some(size) = entry.size {
                        println!("{:.3}\t{}\t{}", entry.score, size, entry.filename);
                    } else {
                        println!("{:.3}\t{}", entry.score, entry.filename);
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{format_secs, human_size};

    #[test]
    fn human_size_bytes() {
        assert_eq!(human_size(0), "0B");
        assert_eq!(human_size(1), "1B");
        assert_eq!(human_size(1023), "1023B");
    }

    #[test]
    fn human_size_three_sig_figs() {
        assert_eq!(human_size(1024), "1.00K");
        assert_eq!(human_size(1536), "1.50K"); // 1.5 KiB
        assert_eq!(human_size(10 * 1024), "10.0K");
        assert_eq!(human_size(100 * 1024), "100K");
        assert_eq!(human_size(700 * 1024 * 1024), "700M");
    }

    #[test]
    fn human_size_promotes_on_rounding() {
        // 1023 MiB stays in M (top of the unit's range).
        assert_eq!(human_size(1023 * 1024 * 1024), "1023M");
        // A value that rounds to 1024 promotes to the next unit.
        let almost_gib = (1023.7 * 1024.0 * 1024.0) as u64;
        assert_eq!(human_size(almost_gib), "1.00G");
        // 1024 MiB is exactly 1 GiB.
        assert_eq!(human_size(1024 * 1024 * 1024), "1.00G");
    }

    #[test]
    fn human_size_large_units() {
        assert_eq!(human_size(1024u64.pow(4)), "1.00T");
        assert_eq!(human_size(1024u64.pow(5)), "1.00P");
        // Caps at the largest unit instead of overflowing.
        assert_eq!(human_size(5 * 1024u64.pow(5)), "5.00P");
    }

    #[test]
    fn format_secs_compact_duration() {
        assert_eq!(format_secs(0.0), "-");
        assert_eq!(format_secs(-1.0), "-");
        assert_eq!(format_secs(f64::NAN), "-");
        assert_eq!(format_secs(45.0), "45s");
        assert_eq!(format_secs(90.0), "1m30s");
        assert_eq!(format_secs(5400.0), "1h30m0s");
        // Rounds to the nearest second.
        assert_eq!(format_secs(59.6), "1m0s");
    }
}
