#![allow(dead_code)]
#![allow(unused_imports)]

mod bloom;
mod classifier;
mod ngrams;
mod normalize;
mod playlist;
mod tokenize;
mod tokens;
mod walk;

use ahash::AHashSet;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use classifier::{Classifier, FileSizeClassifier, DirSizeClassifier, NaiveBayesClassifier};
use crate::ngrams::{Ngram,Ngrams};
use crate::playlist::{Playlist, M3uPlaylist};
use crate::tokenize::PairTokenizer;
use crate::tokens::{Pair, Token, TokenMap, Tokens};
use crate::walk::Walk;
use humansize::{format_size, BINARY};
use log::*;
use rayon::ThreadPool;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{channel, Receiver, Sender};
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

//#[derive(Debug)]
//struct State {
//    path: PathBuf,
//    contents: Vec<String>,
//}
//
//impl State {
//    fn new(path: &Path) -> State {
//        State {
//            path: path.to_owned(),
//            contents: Vec::new(),
//        }
//    }
//
//    fn load(&mut self) -> io::Result<()> {
//        match File::open(&self.path) {
//            Ok(file) => {
//                let reader = io::BufReader::new(file);
//                for line in reader.lines().map_while(Result::ok) {
//                    self.contents.push(line);
//                }
//                Ok(())
//            }
//            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
//            Err(e) => Err(e),
//        }
//    }
//
//    fn from(path: &Path) -> io::Result<State> {
//        let mut state = State::new(path);
//        state.load()?;
//        Ok(state)
//    }
//
//    fn update(&mut self, line: &str) -> io::Result<()> {
//        self.contents.push(line.to_owned());
//        let mut file = OpenOptions::new()
//            .create(true)
//            .append(true)
//            .open(&self.path)?;
//        writeln!(file, "{}", line)?;
//        Ok(())
//    }
//
//    fn iter(&self) -> impl Iterator<Item = PathBuf> + '_ {
//        self.contents.iter().map(PathBuf::from)
//    }
//}
//
//#[derive(Debug, Default)]
//struct FileState {
//    path: PathBuf,
//    // Classifier state.
//    ngrams: Vec<Ngram>,
//    classifier_score: f64,
//    // File size state.
//    file_size: u64,
//    file_size_score: f64,
//
//    score: f64,
//}

//impl FileState {
//    fn new(
//        path: PathBuf,
//        ngrams: Vec<Ngram>,
//        file_size: u64,
//        file_size_log_base: Option<f64>,
//    ) -> Self {
//        let file_size_score = if let Some(base) = file_size_log_base {
//            ((file_size + 1) as f64).log(base)
//        } else {
//            0.0
//        };
//        Self {
//            path,
//            ngrams,
//            file_size,
//            file_size_score,
//            classifier_score: 0.0,
//            score: 0.0,
//        }
//    }
//
//    fn update(&mut self, classifier: &NaiveBayesClassifier) {
//        self.classifier_score = classifier.predict_delete(&self.ngrams);
//        self.score = self.file_size_score + self.classifier_score;
//    }
//
//    fn debug(&self, tokenizer: &Tokenizer, classifier: &NaiveBayesClassifier) {
//        #[derive(Debug)]
//        #[allow(dead_code)]
//        struct Current<'a> {
//            path: &'a Path,
//            size: String,
//            classifier_score: f64,
//            file_size_score: f64,
//            ngrams: Vec<(f64, String)>,
//        }
//        let debug = Current {
//            path: &self.path,
//            size: format_size(self.file_size, BINARY),
//            classifier_score: round(self.classifier_score),
//            file_size_score: round(self.file_size_score),
//            ngrams: classifier.debug_delete(tokenizer, &self.ngrams),
//        };
//        println!("{:?}", debug);
//    }
//}

//struct FileTokens {
//    file: PathBuf,
//    tokens: Tokens,
//}

//fn parallel_tokenize_files(
//    pool: &ThreadPool,
//    tokenizer: &Arc<Tokenizer>,
//    files: Vec<PathBuf>,
//) -> Vec<FileTokens> {
//    let total_count = files.len();
//    assert!(files.len() > 0);
//
//    let (tx, rx) = channel();
//
//    for file in files {
//        let tx = tx.clone();
//        let tokenizer = tokenizer.clone();
//        pool.spawn(move || {
//            let mut hashes: Vec<Token> = tokenizer.tokenize(&file, None).into_iter().collect();
//            hashes.sort();
//            hashes.dedup();
//            let tokens = Tokens::from(hashes);
//            tx.send(FileTokens { file, tokens }).unwrap();
//        });
//    }
//
//    drop(tx);
//
//    let mut ret = Vec::new();
//    let mut last = SystemTime::now();
//    while let Ok(file_tokens) = rx.recv() {
//        let progress = (ret.len() as f64 / total_count as f64) * 100.0;
//
//        let now = SystemTime::now();
//        if now.duration_since(last).unwrap() > Duration::from_millis(100) {
//            last = now;
//            info!(
//                "Progress {:.2} Completed {:?} tokens {:?}",
//                progress,
//                file_tokens.file,
//                file_tokens.tokens.len()
//            );
//        }
//
//        ret.push(file_tokens);
//    }
//
//    ret
//}

//use std::cmp::Ordering;
//
//type TokenCount = (u64, u32);
//
//fn merge_sum(a: &[TokenCount], b: &[TokenCount]) -> Vec<TokenCount> {
//    let (a_len, b_len) = (a.len(), b.len());
//    let mut ret: Vec<(u64, u32)> = Vec::with_capacity(a_len.max(b_len));
//    let mut i = 0;
//    let mut j = 0;
//    while i < a_len && j < b_len {
//        let (a_hash, a_count) = a[i];
//        let (b_hash, b_count) = b[j];
//        match a_hash.cmp(&b_hash) {
//            Ordering::Less => {
//                ret.push((a_hash, a_count));
//                i += 1;
//            }
//            Ordering::Equal => {
//                ret.push((a_hash, a_count.saturating_add(b_count)));
//                i += 1;
//                j += 1;
//            }
//            Ordering::Greater => {
//                ret.push((b_hash, b_count));
//                j += 1;
//            }
//        }
//    }
//    ret.extend_from_slice(&a[i..]);
//    ret.extend_from_slice(&b[j..]);
//    ret
//}

//fn parallel_token_count(
//    pool: &ThreadPool,
//    mut token_counts: Vec<Vec<(u64, u32)>>,
//) -> Vec<(u64, u32)> {
//    while token_counts.len() > 1 {
//        println!("token counts len {}", token_counts.len());
//
//        let (tx, rx) = channel();
//        while let Some(a) = token_counts.pop() {
//            if let Some(b) = token_counts.pop() {
//                let tx = tx.clone();
//                pool.spawn(move || {
//                    tx.send(merge_sum(&a, &b)).unwrap();
//                });
//            } else {
//                token_counts.push(a);
//                break;
//            }
//        }
//
//        drop(tx);
//        while let Ok(counts) = rx.recv() {
//            token_counts.push(counts);
//        }
//    }
//
//    token_counts.into_iter().next().unwrap()
//}

//fn parallel_unique_tokens(pool: &ThreadPool, tokens_vec: Vec<Tokens>) -> (Tokens, Tokens) {
//    let mut unique_vec: Vec<Tokens> = tokens_vec;
//    let mut common_vec: Vec<Tokens> = Vec::new();
//
//    loop {
//        let (tx, rx) = channel();
//        println!(
//            "unique_vec {}, common_vec {}",
//            unique_vec.len(),
//            common_vec.len()
//        );
//
//        while let Some(a) = unique_vec.pop() {
//            if let Some(b) = unique_vec.pop() {
//                let tx = tx.clone();
//                pool.spawn(move || {
//                    let unique = a.symmetric_difference(&b);
//                    let common = a.intersection(&b);
//                    tx.send((Some(unique), Some(common))).unwrap();
//                });
//            } else {
//                unique_vec.push(a);
//                break;
//            }
//        }
//
//        while let Some(a) = common_vec.pop() {
//            if let Some(b) = common_vec.pop() {
//                let tx = tx.clone();
//                pool.spawn(move || {
//                    let common = a.union(&b);
//                    tx.send((None, Some(common))).unwrap();
//                });
//            } else {
//                common_vec.push(a);
//                break;
//            }
//        }
//
//        // Drop last tx handle.
//        drop(tx);
//
//        // Collect for next round.
//        while let Ok((unique, common)) = rx.recv() {
//            if let Some(unique) = unique {
//                unique_vec.push(unique);
//            }
//            if let Some(common) = common {
//                common_vec.push(common);
//            }
//        }
//
//        // Finish.
//        if unique_vec.len() == 1 && common_vec.len() == 1 {
//            let unique = unique_vec.into_iter().next().unwrap();
//            let common = common_vec.into_iter().next().unwrap();
//            println!(
//                "parallel_unique_tokens unique {} common {}",
//                unique.len(),
//                common.len()
//            );
//            return (unique, common);
//        }
//    }
//}

//fn start_web_server() {
//    std::thread::spawn(move || {
//        println!("Listening on http://localhost:9111/");
//        use rouille::*;
//        rouille::start_server("localhost:9111", move |request| {
//            router!(request,
//                (GET) (/) => {
//                    Response::text("hello world")
//                },
//                _ => Response::empty_404()
//            )
//        });
//    });
//}


#[derive(Debug)]
struct Entry {
    file: walk::File,
    norm: String,
    tokens: Option<Tokens>,
    ngrams: Option<Ngrams>,
    scores: Vec<f64>,  // One score per classifier
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
        let walk = Walk::new(self.args.video_exts.iter().map(String::as_ref));
        for dir in &self.args.dirs {
            walk.walk_dir(dir);
        }

        let rx = walk.into_rx();
        while let Ok(file) = rx.recv() {
            debug!("{:?}", file);

            let file_path: PathBuf = file.dir.join(&file.file_name);
            let norm = normalize::normalize(&file_path);

            let entry = Entry {
                file,
                norm,
                tokens: None,
                ngrams: None,
                scores: Vec::new(),
            };

            self.entries.push(entry);
        }
    }

    fn process_tokens_and_ngrams(&mut self) -> io::Result<()> {
        // Collect all paths that need tokenization
        let mut paths = HashSet::new();
        
        // Add paths from walk results (candidates)
        paths.extend(self.entries.iter().map(|e| e.norm.to_string()));
        
        // Add paths from playlist classifications
        paths.extend(self.playlist.positives().iter().map(|p| normalize::normalize(p)));
        paths.extend(self.playlist.negatives().iter().map(|p| normalize::normalize(p)));

        // Create tokenizer from all paths
        self.tokenizer = Some(tokenize::PairTokenizer::new(paths.iter().map(String::as_str)));
        let tokenizer = self.tokenizer.as_ref().unwrap();

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
                .collect()
        );

        info!("frequent ngrams {:?}", self.frequent_ngrams.as_ref().unwrap().len());

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
            let tokens = self.tokenizer.as_ref().unwrap().tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            let entry = Entry {
                file: walk::File { dir: Arc::new(path.parent().unwrap_or(Path::new("")).to_path_buf()), 
                                 file_name: path.file_name().unwrap().to_string_lossy().to_string(),
                                 size: 0, 
                                 inode: 0 },
                norm,
                tokens: Some(tokens),
                ngrams: Some(temp_ngrams.clone()),
                score: 0.0,
            };
            self.naive_bayes.train_positive(&entry);
        }

        // Process negative examples
        for path in self.playlist.negatives() {
            let norm = normalize::normalize(path);
            let tokens = self.tokenizer.as_ref().unwrap().tokenize(&norm);
            temp_ngrams.windows(&tokens, 5, None, None);
            let entry = Entry {
                file: walk::File { dir: Arc::new(path.parent().unwrap_or(Path::new("")).to_path_buf()), 
                                 file_name: path.file_name().unwrap().to_string_lossy().to_string(),
                                 size: 0, 
                                 inode: 0 },
                norm,
                tokens: Some(tokens),
                ngrams: Some(temp_ngrams.clone()),
                score: 0.0,
            };
            self.naive_bayes.train_negative(&entry);
        }

        Ok(())
    }

    fn process_classifiers(&mut self) {
        // Initialize score vectors in entries
        let classifier_count = self.classifiers.len() + 1;
        for entry in &mut self.entries {
            entry.scores = vec![0.0; classifier_count];
        }

        // Calculate scores for each classifier
        for (idx, classifier) in &mut self.classifiers.iter_mut().enumerate() {
            classifier.calculate_scores(&mut self.entries, idx);
        }
        
        // Calculate naive bayes scores
        self.naive_bayes.calculate_scores(&mut self.entries, self.classifiers.len());

        // Sort entries in place by average score descending
        self.entries.sort_by(|a, b| {
            let a_avg = a.scores.iter().sum::<f64>() / classifier_count as f64;
            let b_avg = b.scores.iter().sum::<f64>() / classifier_count as f64;
            b_avg.partial_cmp(&a_avg).unwrap()
        });
    }

    fn run(&mut self) -> io::Result<()> {
        self.init_thread_priority();
        self.collect_files();
        self.process_tokens_and_ngrams()?;
        self.process_classifiers();
        Ok(())
    }
}

fn main() -> io::Result<()> {
    let mut app = App::new()?;
    app.run()?;
    Ok(())


    //let mut delete = State::from(&args.delete)?;
    //for path in delete.iter() {
    //    let ngrams = tokenizer.ngrams_cached(&path);
    //    classifier.train_delete(&ngrams);
    //    file_sizes.remove(&path);
    //}

    //let mut keep = State::from(&args.keep)?;
    //for path in keep.iter() {
    //    let ngrams = tokenizer.ngrams_cached(&path);
    //    classifier.train_keep(&ngrams);
    //    file_sizes.remove(&path);
    //}

    //let mut files_vec: Vec<FileState> = Vec::new();
    //for (path, size) in file_sizes.into_iter() {
    //    let ngrams = tokenizer.ngrams_cached(&path);
    //    files_vec.push(FileState::new(path, ngrams, size, args.file_size_log_base));
    //}

    //while !files_vec.is_empty() {
    //    for file in files_vec.iter_mut() {
    //        file.update(&classifier);
    //    }

    //    files_vec.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    //    println!();
    //    {
    //        let mut xmin = 0.0;
    //        let mut xmax = 0.0;
    //        let mut ymin = 0.0;
    //        let mut ymax = 0.0;
    //        let mut points = Vec::new();
    //        for (i, file) in files_vec.iter().enumerate() {
    //            let (x, y) = (i as f32, file.file_size_score as f32);
    //            xmin = f32::min(xmin, x);
    //            xmax = f32::max(xmax, x);
    //            ymin = f32::min(ymin, y);
    //            ymax = f32::max(ymax, y);
    //            points.push((x, y));
    //        }
    //        println!("File size scores");
    //        Chart::new_with_y_range(300, 80, xmin, xmax, ymin, ymax)
    //            .lineplot(&Shape::Points(&points))
    //            .nice();
    //    }

    //    {
    //        let mut xmin = 0.0;
    //        let mut xmax = 0.0;
    //        let mut ymin = 0.0;
    //        let mut ymax = 0.0;
    //        let mut points = Vec::new();
    //        println!("Classifier scores");
    //        for (i, file) in files_vec.iter().enumerate() {
    //            let (x, y) = (i as f32, file.classifier_score as f32);
    //            xmin = f32::min(xmin, x);
    //            xmax = f32::max(xmax, x);
    //            ymin = f32::min(ymin, y);
    //            ymax = f32::max(ymax, y);
    //            points.push((x, y));
    //        }
    //        Chart::new_with_y_range(300, 80, xmin, xmax, ymin, ymax)
    //            .lineplot(&Shape::Points(&points))
    //            .nice();
    //    }

    //    let file_state = files_vec.pop().unwrap();

    //    file_state.debug(&tokenizer, &classifier);

    //    let file_name = file_state
    //        .path
    //        .file_name()
    //        .unwrap()
    //        .to_string_lossy()
    //        .to_string();

    //    let path_str = file_state.path.to_string_lossy().to_string();

    //    let vlc = VLCProcessHandle::new(&args, &file_state.path);
    //    match vlc.wait_for_status() {
    //        Ok(status) => {
    //            let found_file_name = status.file_name();
    //            if Some(&file_name) != found_file_name.as_ref() {
    //                error!(
    //                    "Filename mismatch {:?} {:?}, skipping",
    //                    file_name, found_file_name
    //                );
    //                continue;
    //            }
    //        }
    //        Err(e) => {
    //            error!("Vlc startup error {:?}", e);
    //            continue;
    //        }
    //    }

    //    loop {
    //        std::thread::sleep(std::time::Duration::from_millis(100));

    //        let status = match vlc.status() {
    //            Ok(status) => {
    //                debug!("{:?}", status);
    //                status
    //            }
    //            Err(e) => {
    //                error!("Status error: {:?}", e);
    //                break;
    //            }
    //        };

    //        match status.state() {
    //            "stopped" => {
    //                delete.update(&path_str)?;
    //                classifier.train_delete(&file_state.ngrams);
    //                info!("{:?} (DELETE)", path_str);
    //                break;
    //            }
    //            "paused" => {
    //                keep.update(&path_str)?;
    //                classifier.train_keep(&file_state.ngrams);
    //                info!("{:?} (KEEP)", path_str);
    //                break;
    //            }
    //            _ => {}
    //        }
    //    }
    //}

    //loop {
    //    std::thread::sleep(std::time::Duration::from_secs(1));
    //}
    //
}
