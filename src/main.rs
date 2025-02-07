#![allow(dead_code)]
#![allow(unused_imports)]

mod bloom;
mod classifier;
mod ngrams;
mod normalize;
mod playlist;
mod tokenize;
mod tokens;
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
use std::collections::{BTreeMap, HashMap};
use std::collections::HashSet;
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
    /// M3U playlist file for storing classifications
    playlist: PathBuf,

    /// Directories to scan for video files (defaults to current directory)
    #[arg(default_value = ".")]
    dirs: Vec<PathBuf>,

    #[clap(long, default_value = "info")]
    log_level: String,

    /// Fullscreen VLC playback.
    #[clap(short, long)]
    fullscreen: bool,

    #[clap(long, default_value_t = 9111)]
    port: u16,

    #[clap(long, default_value = "9010")]
    vlc_port: u16,

    #[arg(
        long,
        value_delimiter = ',',
        default_value = "avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4"
    )]
    video_exts: Vec<String>,
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
    entries: Vec<Entry>,
    tokenizer: Option<PairTokenizer>,
    frequent_ngrams: Option<ahash::AHashSet<Ngram>>,
    classifiers: Vec<Box<dyn Classifier>>,
    naive_bayes: NaiveBayesClassifier,
    playlist: M3uPlaylist,
}

impl App {
    fn new() -> io::Result<Self> {
        let args = Args::parse();
        if std::env::var("RUST_LOG").is_err() {
            std::env::set_var("RUST_LOG", &args.log_level);
        }
        env_logger::init();
        info!("{:#?}", args);

        // Initialize playlist
        let playlist = M3uPlaylist::open(args.playlist.clone())?;

        // Create default classifiers
        let mut classifiers: Vec<Box<dyn Classifier>> = Vec::new();
        classifiers.push(Box::new(FileSizeClassifier::new(2.0, false)));
        classifiers.push(Box::new(DirSizeClassifier::new(2.0, false)));

        // Create naive bayes classifier separately
        let naive_bayes = NaiveBayesClassifier::new(false);

        Ok(Self {
            args,
            entries: Vec::new(),
            tokenizer: None,
            frequent_ngrams: None,
            classifiers,
            naive_bayes,
            playlist,
        })
    }

    fn init_thread_priority(&self) {
        rayon::broadcast(|_| {
            set_current_thread_priority(ThreadPriority::Min).unwrap();
        });
    }

    fn collect_files(&mut self) {
        // Create set of already classified paths
        let mut classified = HashSet::new();
        classified.extend(self.playlist.positives().iter().cloned());
        classified.extend(self.playlist.negatives().iter().cloned());
        info!("Classified {:?}", classified);

        let walk = Walk::new(self.args.video_exts.iter().map(String::as_ref));
        for dir in &self.args.dirs {
            walk.walk_dir(dir);
        }

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
            // Initialize entry with scores array sized for all classifiers plus naive bayes
            let entry = Entry {
                file,
                norm,
                tokens: None,
                ngrams: None,
                scores: vec![0.0; self.classifiers.len() + 1].into_boxed_slice(),
            };

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

        info!("tokenizer tokens {:?}", tokenizer.count());

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

        // Process bounds for each classifier
        for classifier in &mut self.classifiers {
            classifier.process_entries(&self.entries);
        }
        self.naive_bayes.process_entries(&self.entries);

        // Calculate raw scores for each classifier
        for (idx, classifier) in self.classifiers.iter().enumerate() {
            for entry in &mut self.entries {
                entry.scores[idx] = classifier.calculate_score(entry);
            }
        }

        // Calculate naive bayes scores
        for entry in &mut self.entries {
            entry.scores[classifier_count - 1] = self.naive_bayes.calculate_score(entry);
        }

        // Normalize each column of scores
        for col in 0..classifier_count {
            let col_scores: Vec<f64> = self.entries.iter().map(|e| e.scores[col]).collect();
            if let (Some(min), Some(max)) = (
                col_scores.iter().copied().reduce(f64::min),
                col_scores.iter().copied().reduce(f64::max),
            ) {
                if (max - min).abs() > f64::EPSILON {
                    for (entry, &raw_score) in self.entries.iter_mut().zip(&col_scores) {
                        entry.scores[col] = (raw_score - min) / (max - min);
                    }
                }
            }
        }

        // Sort entries by total score descending
        debug!("Sorting {} entries", self.entries.len());
        self.entries.sort_by(|a, b| {
            let a_sum = a.scores.iter().sum::<f64>();
            let b_sum = b.scores.iter().sum::<f64>();
            debug!("Comparing scores: {} vs {}", a_sum, b_sum);
            b_sum.partial_cmp(&a_sum).expect("Invalid score comparison")
        });
    }

    fn run(&mut self) -> io::Result<()> {
        self.init_thread_priority();
        self.collect_files();
        self.process_tokens_and_ngrams()?;

        // Main classification loop
        while !self.entries.is_empty() {
            self.process_classifiers();

            // Get highest scoring entry
            if let Some(entry) = self.entries.first() {
                let path = entry.file.dir.join(&entry.file.file_name);
                let file_name: Option<String> = Some(entry.file.file_name.to_string_lossy().to_string());

                // Build score string with classifier names
                let score_details: Vec<String> = self.classifiers.iter()
                    .enumerate()
                    .map(|(i, c)| format!("{}: {:.3}", c.name(), entry.scores[i]))
                    .chain(std::iter::once(format!("naive_bayes: {:.3}", entry.scores.last().unwrap())))
                    .collect();
                info!("Top candidate: {:?}\nScores: {}", path, score_details.join(", "));
                
                // Start VLC for classification
                let vlc = vlc::VLCProcessHandle::new(&self.args, &path);
                match vlc.wait_for_status() {
                    Ok(status) => {
                        let found_file_name: Option<String> = status.file_name();
                        if file_name != found_file_name {
                            error!(
                                "Filename mismatch {:?} {:?}, skipping",
                                file_name, found_file_name
                            );
                            continue;
                        }
                    }
                    Err(e) => {
                        error!("VLC startup error {:?}", e);
                        continue;
                    }
                }

                // Wait for user classification via VLC controls
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(100));

                    let status = match vlc.status() {
                        Ok(status) => {
                            debug!("{:?}", status);
                            status
                        }
                        Err(e) => {
                            error!("Status error: {:?}", e);
                            break;
                        }
                    };

                    match status.state() {
                        "stopped" => {
                            let entry = self.entries.remove(0);
                            self.playlist.add_positive(&path)?;
                            self.naive_bayes.train_negative(entry.ngrams.as_ref().unwrap());
                            info!("{:?} (NEGATIVE)", path);
                            break;
                        }
                        "paused" => {
                            let entry = self.entries.remove(0);
                            self.playlist.add_negative(&path)?;
                            self.naive_bayes.train_positive(entry.ngrams.as_ref().unwrap());
                            info!("{:?} (POSITIVE)", path);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let mut app = App::new()?;
    app.run()?;
    Ok(())


}
