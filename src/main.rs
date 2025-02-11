#![allow(dead_code)]
#![allow(unused_imports)]

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
use crate::playlist::{M3uPlaylist, Playlist};
use crate::tokenize::PairTokenizer;
use crate::tokens::{Pair, Token, TokenMap, Tokens};
use crate::walk::Walk;
use ahash::AHashSet;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use classifier::{Classifier, DirSizeClassifier, FileSizeClassifier, NaiveBayesClassifier};
use humansize::{format_size, BINARY};
use log::*;
use rayon::prelude::*;
use rayon::ThreadPool;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};
use textplots::{Chart, Plot, Shape};
use thread_priority::*;

#[derive(Debug)]
enum Error {
    Reqwest(reqwest::Error),
    SerdeJson(serde_json::Error),
    Timeout,
    FilenameMismatch,
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Reqwest(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::SerdeJson(e)
    }
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
    Build {
        /// M3U playlist file for storing classifications
        playlist: PathBuf,
        #[command(flatten)]
        args: BuildArgs,
    },
    /// List positively classified files
    Positive {
        /// M3U playlist file
        playlist: PathBuf,
    },
    /// List negatively classified files
    Negative {
        /// M3U playlist file
        playlist: PathBuf,
    },
}

#[derive(Parser, Debug, Clone)]
struct BuildArgs {
    /// Directories to scan for video files
    dirs: Vec<PathBuf>,
    /// Video file extensions to scan for
    video_exts: Vec<String>,
    #[command(flatten)]
    vlc: VlcArgs,
    #[command(flatten)]
    file_size: FileSizeArgs,
    #[command(flatten)]
    dir_size: DirSizeArgs,
}

#[derive(Parser, Debug, Clone)]
struct VlcArgs {
    /// Fullscreen VLC playback
    #[clap(long)]
    fullscreen: bool,
    /// VLC HTTP interface port
    #[clap(long, default_value_t = 9111)]
    port: u16,
    /// Timeout in seconds for VLC startup
    #[clap(long, default_value_t = 60)]
    vlc_timeout: u64,
}

#[derive(Parser, Debug, Clone)]
struct FileSizeArgs {
    /// Bias scoring based on file sizes
    #[clap(long, default_value_t = 0.0)]
    file_size_bias: f64,
}

#[derive(Parser, Debug, Clone)]
struct DirSizeArgs {
    /// Bias scoring based on directory sizes
    #[clap(long, default_value_t = 0.0)]
    dir_size_bias: f64,
}


impl Default for BuildArgs {
    fn default() -> Self {
        Self {
            dirs: vec![PathBuf::from(".")],
            video_exts: "avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4"
                .split(',')
                .map(String::from)
                .collect(),
            vlc: VlcArgs::default(),
            file_size: FileSizeArgs::default(),
            dir_size: DirSizeArgs::default(),
        }
    }
}

impl Default for VlcArgs {
    fn default() -> Self {
        Self {
            fullscreen: false,
            port: 9111,
            vlc_timeout: 60,
        }
    }
}

impl Default for FileSizeArgs {
    fn default() -> Self {
        Self {
            file_size_bias: 0.0,
        }
    }
}

impl Default for DirSizeArgs {
    fn default() -> Self {
        Self {
            dir_size_bias: 0.0,
        }
    }
}

#[derive(Debug)]
struct Entry {
    file: walk::File,
    norm: String,
    tokens: Option<Tokens>,
    ngrams: Option<Ngrams>,
    scores: Box<[f64]>, // One score per classifier
}

struct App {
    args: Args,
    build_args: BuildArgs,
    entries: Vec<Entry>,
    tokenizer: Option<PairTokenizer>,
    frequent_ngrams: Option<ahash::AHashSet<Ngram>>,
    file_size_classifier: Option<FileSizeClassifier>,
    dir_size_classifier: Option<DirSizeClassifier>,
    naive_bayes: NaiveBayesClassifier,
    playlist: M3uPlaylist,
    visualizer: viz::ScoreVisualizer,
}

impl App {
    fn new(args: Args, build_args: BuildArgs, playlist: M3uPlaylist) -> io::Result<Self> {
        info!("{:#?}", args);

        // Initialize visualizer
        let visualizer = viz::ScoreVisualizer::default();

        // Initialize optional classifiers based on args
        let file_size_classifier = if build_args.file_size.file_size_bias != 0.0 {
            let log_base = build_args.file_size.file_size_bias.abs();
            assert!(log_base > 1.0);
            let reverse = build_args.file_size.file_size_bias < 0.0;
            Some(FileSizeClassifier::new(log_base, reverse))
        } else {
            None
        };

        let dir_size_classifier = if build_args.dir_size.dir_size_bias != 0.0 {
            let log_base = build_args.dir_size.dir_size_bias.abs();
            assert!(log_base > 1.0);
            let reverse = build_args.dir_size.dir_size_bias < 0.0;
            Some(DirSizeClassifier::new(log_base, reverse))
        } else {
            None
        };

        Ok(Self {
            args,
            build_args,
            entries: Vec::new(),
            tokenizer: None,
            frequent_ngrams: None,
            file_size_classifier,
            dir_size_classifier,
            naive_bayes: NaiveBayesClassifier::new(false),
            playlist,
            visualizer,
        })
    }

    fn init_thread_priority(&self) {
        rayon::broadcast(|_| {
            set_current_thread_priority(ThreadPriority::Min).unwrap();
        });
    }

    fn classifiers(&self) -> Vec<&dyn Classifier> {
        let mut classifiers = Vec::new();
        if let Some(ref c) = self.file_size_classifier {
            classifiers.push(c as &dyn Classifier);
        }
        if let Some(ref c) = self.dir_size_classifier {
            classifiers.push(c as &dyn Classifier);
        }
        classifiers.push(&self.naive_bayes as &dyn Classifier);
        classifiers
    }

    fn collect_files(&mut self) {
        // Create set of already classified paths
        let mut classified = HashSet::new();
        classified.extend(self.playlist.positives().iter().cloned());
        classified.extend(self.playlist.negatives().iter().cloned());
        //info!("Classified {:?}", classified);

        let walk = Walk::new(self.build_args.video_exts.iter().map(String::as_ref));
        for dir in &self.build_args.dirs {
            walk.walk_dir(dir);
        }

        let classifiers_len = self.classifiers().len();

        let rx = walk.into_rx();
        while let Ok(file) = rx.recv() {
            debug!("{:?}", file);

            let file_path = file.dir.join(&file.file_name);

            // Skip if already classified
            if classified.contains(&file_path) {
                debug!("Skipping already classified file: {:?}", file_path);
                continue;
            }

            let norm = normalize::normalize(&file_path);

            let mut scores = vec![0.0; classifiers_len];
            scores.shrink_to_fit();

            // Initialize entry with scores array sized for all classifiers plus naive bayes
            let entry = Entry {
                file,
                norm,
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

    fn process_tokens_and_ngrams(&mut self) -> io::Result<()> {
        // Collect all paths that need tokenization
        let mut paths = HashSet::new();

        // Add paths from walk results (candidates)
        paths.extend(self.entries.iter().map(|e| e.norm.to_string()));

        // Add paths from playlist classifications
        paths.extend(
            self.playlist
                .positives()
                .iter()
                .map(|p| normalize::normalize(p)),
        );
        paths.extend(
            self.playlist
                .negatives()
                .iter()
                .map(|p| normalize::normalize(p)),
        );

        // Create tokenizer from all paths
        self.tokenizer = Some(tokenize::PairTokenizer::new(
            paths.iter().map(String::as_str),
        ));
        let tokenizer = self.tokenizer.as_ref().unwrap();

        info!("tokenizer tokens {:?}", tokenizer.token_map().count());

        // Process all paths to find frequent ngrams
        let mut ngram_counts: ahash::AHashMap<Ngram, u8> = ahash::AHashMap::new();
        let mut temp_ngrams = Ngrams::default();

        // Count ngrams from all sources
        for path in &paths {
            let tokens = tokenizer.tokenize(path);
            temp_ngrams.windows(&tokens, 5, None, None);
            for ngram in temp_ngrams.iter() {
                let e = ngram_counts.entry(*ngram).or_default();
                *e = e.saturating_add(1);
            }
        }

        info!("total ngrams {:?}", ngram_counts.len());

        // Filter to frequent ngrams
        self.frequent_ngrams = Some(
            ngram_counts
                .into_iter()
                .filter_map(|(ngram, count)| if count > 1 { Some(ngram) } else { None })
                .collect(),
        );

        info!(
            "frequent ngrams {:?}",
            self.frequent_ngrams.as_ref().unwrap().len()
        );

        // Final pass to store tokens and frequent ngrams for candidates only
        for e in self.entries.iter_mut() {
            e.tokens = Some(tokenizer.tokenize(&e.norm));

            let mut ngrams = Ngrams::default();
            ngrams.windows(
                e.tokens.as_ref().unwrap(),
                5,
                self.frequent_ngrams.as_ref(),
                None,
            );
            e.ngrams = Some(ngrams);
        }

        // Train naive bayes classifier on playlist entries
        let mut temp_ngrams = Ngrams::default();

        // Process positive examples
        for path in self.playlist.positives() {
            let norm = normalize::normalize(path);
            let tokens = tokenizer.tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            self.naive_bayes.train_positive(&temp_ngrams);
        }

        // Process negative examples
        for path in self.playlist.negatives() {
            let norm = normalize::normalize(path);
            let tokens = tokenizer.tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            self.naive_bayes.train_negative(&temp_ngrams);
        }

        Ok(())
    }

    fn process_classifiers(&mut self) {
        // Create temporary vector to swap with entries
        let mut temp_entries = Vec::new();
        std::mem::swap(&mut self.entries, &mut temp_entries);

        let classifiers = self.classifiers();

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

    // Gets classification from user via VLC
    fn get_user_classification(&self, entry: &Entry) -> io::Result<Option<vlc::Classification>> {
        let path = entry.file.dir.join(&entry.file.file_name);
        let file_name = Some(entry.file.file_name.to_string_lossy().to_string());

        // Start VLC and get classification
        let vlc = vlc::VLCProcessHandle::new(&self.build_args.vlc, &path, file_name);

        // Wait for VLC to start and verify filename
        if let Err(e) = vlc.wait_for_status(self.build_args.vlc_timeout) {
            error!("VLC startup error {:?}", e);
            return Ok(None);
        }

        match vlc.get_classification() {
            Ok(classification) => {
                if matches!(classification, vlc::Classification::Skipped) {
                    error!("Classification skipped for {:?}", path);
                    Ok(None)
                } else {
                    Ok(Some(classification))
                }
            }
            Err(e) => {
                error!("Classification error: {:?}", e);
                Ok(None)
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
                5,
                self.frequent_ngrams.as_ref(),
                Some(&mut ngram_tokens),
            );
            ngram_tokens.sort();
            ngram_tokens.dedup();
        }

        let mut tuples = Vec::new();
        for window in ngram_tokens.into_iter() {
            let ngram = Ngram::new(&window);
            let score = self.naive_bayes.ngram_score(ngram);
            tuples.push((window, score));
        }

        // Sort tuples by absolute score descending
        tuples.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());

        // Display top 50 ngrams by absolute score
        println!("Top ngrams by absolute score:");
        for (tokens, score) in tuples.iter().take(50) {
            let token_strs: Vec<&str> = tokens
                .iter()
                .map(|t| token_map.get_str(*t).unwrap())
                .collect();
            print!("{:?}: {:.3}, ", token_strs, score);
        }
        println!();

        // Display classifier scores
        let score_details: Vec<String> = self
            .classifiers()
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}: {:.3}", c.name(), entry.scores[i]))
            .collect();

        info!("Classifier scores: {}", score_details.join(", "));
    }

    // Handles the classification result
    fn handle_classification(
        &mut self,
        entry: Entry,
        classification: vlc::Classification,
    ) -> io::Result<()> {
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
            vlc::Classification::Skipped => unreachable!(), // Handled in get_user_classification
        }
        Ok(())
    }

    // Main entry point remains simple and high-level
    fn run(&mut self) -> io::Result<()> {
        self.init_thread_priority();
        self.collect_files();
        self.process_tokens_and_ngrams()?;
        self.classification_loop()?;
        Ok(())
    }

    // Handles the main classification loop
    fn classification_loop(&mut self) -> io::Result<()> {
        while !self.entries.is_empty() {
            self.process_classifiers();

            if let Some(entry) = self.entries.last().clone() {
                // Get classifier names
                let classifier_names: Vec<&str> =
                    self.classifiers().iter().map(|c| c.name()).collect();

                // Display detailed information about the entry
                self.display_entry_details(&entry);

                // Display visualizations
                self.visualizer
                    .display_distributions(&self.entries, &entry, &classifier_names);

                let entry = self.entries.pop().unwrap();
                if let Some(classification) = self.get_user_classification(&entry)? {
                    self.handle_classification(entry, classification)?;
                }
            }
        }
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &args.log_level);
    }
    env_logger::init();
    

    match args.command {
        Command::Build { playlist: p, args: build_args } => {
            let playlist = M3uPlaylist::open(p)?;
            let mut app = App::new(args.clone(), build_args, playlist)?;
            app.run()?;
        }
        Command::Positive { playlist } => {
            let playlist = M3uPlaylist::open(playlist)?;
            for path in playlist.positives() {
                println!("{}", path.display());
            }
        }
        Command::Negative { playlist } => {
            let playlist = M3uPlaylist::open(playlist)?;
            for path in playlist.negatives() {
                println!("{}", path.display());
            }
        }
    }
    Ok(())
}
