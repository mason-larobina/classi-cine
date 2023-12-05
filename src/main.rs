use clap::Parser;
use log::*;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use walkdir::WalkDir;

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

#[derive(Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Copy, Clone, Default)]
struct Token(u32);

#[derive(Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Copy, Clone, Default)]
struct Ngram(u32);

#[derive(Debug)]
struct Tokenizer {
    tokenize: Tokenize,

    token_count: u32,
    tokens: HashMap<String, Token>,

    k: usize,
    ngram_count: u32,
    ngrams: HashMap<Vec<Token>, Ngram>,
}

impl Tokenizer {
    fn new(k: usize, tokenize: Tokenize, files: &HashSet<PathBuf>) -> Self {
        assert!(k > 0);

        let file_count = files.len();
        assert!(file_count > 0);

        let mut tokenizer = Self {
            tokenize,

            token_count: 0,
            tokens: HashMap::new(),

            k,
            ngram_count: 0,
            ngrams: HashMap::new(),
        };

        // Unique token count per file.
        let mut token_counts: HashMap<String, usize> = HashMap::new();
        for path in files {
            let mut tokens = tokenizer.tokenize_new(path);
            tokens.sort();
            tokens.dedup();
            for token in tokens {
                let e = token_counts.entry(token).or_default();
                *e += 1;
            }
        }

        let mut unique_tokens: BTreeSet<String> = BTreeSet::new();
        let mut common_tokens: BTreeSet<String> = BTreeSet::new();
        for (token, count) in token_counts {
            if count > 1 {
                tokenizer.make_token(token);
            } else if count == 1 {
                unique_tokens.insert(token);
            } else if count == file_count {
                common_tokens.insert(token);
            }
        }
        debug!("Drop unique tokens: {:?}", unique_tokens);
        debug!("Drop common tokens: {:?}", common_tokens);

        let mut ngram_counts: HashMap<Vec<Token>, usize> = HashMap::new();
        for path in files {
            let ngrams: BTreeSet<Vec<Token>> = tokenizer.ngrams_new(path).into_iter().collect();
            for ngram in ngrams {
                let e = ngram_counts.entry(ngram).or_default();
                *e += 1;
            }
        }

        let mut unique_ngrams: BTreeSet<Vec<Token>> = BTreeSet::new();
        let mut common_ngrams: BTreeSet<Vec<Token>> = BTreeSet::new();
        for (ngram, count) in ngram_counts {
            if count > 1 {
                tokenizer.make_ngram(ngram);
            } else if count == 1 {
                unique_ngrams.insert(ngram);
            } else if count == file_count {
                common_ngrams.insert(ngram);
            }
        }
        debug!("Drop unique ngrams: {:?}", unique_ngrams);
        debug!("Drop common ngrams: {:?}", common_ngrams);

        info!("File count: {}", file_count);
        info!("Token count: {}", tokenizer.token_count);
        info!("Ngram count: {}", tokenizer.ngram_count);

        tokenizer
    }

    fn make_token(&mut self, token: String) -> Token {
        *self.tokens.entry(token).or_insert_with(|| {
            self.token_count += 1;
            Token(self.token_count)
        })
    }

    fn make_ngram(&mut self, ngram: Vec<Token>) -> Ngram {
        *self.ngrams.entry(ngram).or_insert_with(|| {
            self.ngram_count += 1;
            Ngram(self.ngram_count)
        })
    }

    fn tokenize_new(&self, path: &Path) -> Vec<String> {
        let mut path: String = path.to_string_lossy().to_string();
        path.make_ascii_lowercase();

        let mut ret = Vec::new();
        match self.tokenize {
            Tokenize::Words => {
                for token in path
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|word| !word.is_empty())
                {
                    ret.push(token.to_string());
                }
            }
            Tokenize::Chars => {
                for c in path.chars() {
                    if c.is_alphanumeric() || c == '/' {
                        ret.push(c.into());
                        continue;
                    } else if Some(" ") != ret.last().map(|x| x.as_str()) {
                        ret.push(' '.into());
                    }
                }
            }
        }
        ret
    }

    fn tokenize_cached(&self, path: &Path) -> Vec<Token> {
        let mut ret = Vec::new();
        for token in self.tokenize_new(path) {
            ret.push(self.tokens.get(&token).cloned().unwrap_or_default());
        }
        ret
    }

    fn ngrams_new(&self, path: &Path) -> Vec<Vec<Token>> {
        let tokens = self.tokenize_cached(path);
        let mut ret = Vec::new();
        for i in 0..self.k {
            for w in tokens.windows(i + 1) {
                let mut w: Vec<Token> = w.to_vec();
                w.shrink_to_fit();
                ret.push(w);
            }
        }
        ret
    }

    fn ngrams_cached(&self, path: &Path) -> Vec<Ngram> {
        let mut ret = Vec::new();
        for ngram in self.ngrams_new(path) {
            ret.push(self.ngrams.get(&ngram).cloned().unwrap_or_default());
        }
        ret
    }
}

// The NgramCounter struct is designed to maintain counts of ngrams.
#[derive(Debug)]
struct NgramCounter {
    // A HashMap storing the counts of each ngram.
    counts: HashMap<Ngram, usize>,

    // A running total of all ngrams observed.
    total: usize,

    unique_ngram_count: u32,
}

impl NgramCounter {
    fn new(tokenizer: &Tokenizer) -> Self {
        let unique_ngram_count = tokenizer.ngram_count;
        assert!(unique_ngram_count > 0);

        Self {
            counts: HashMap::new(),
            total: 0,
            unique_ngram_count,
        }
    }

    // Increment the count for a given ngram.
    fn inc(&mut self, ngram: Ngram) {
        let e = self.counts.entry(ngram).or_default();
        *e += 1;
        self.total += 1;
    }

    // Get the smoothed log probability of observing a given ngram.
    //
    // Laplace smoothed.
    fn log_p(&self, ngram: &Ngram) -> f64 {
        let count = (self.counts.get(ngram).cloned().unwrap_or_default() + 1) as f64;
        let total = (self.total + self.unique_ngram_count as usize) as f64;
        (count / total).max(f64::MIN_POSITIVE).ln()
    }
}

#[derive(Debug)]
struct NaiveBayesClassifier {
    delete: NgramCounter,
    keep: NgramCounter,
}

impl NaiveBayesClassifier {
    fn new(tokenizer: &Tokenizer) -> Self {
        Self {
            delete: NgramCounter::new(tokenizer),
            keep: NgramCounter::new(tokenizer),
        }
    }

    fn train_delete(&mut self, ngrams: &[Ngram]) {
        for ngram in ngrams {
            self.delete.inc(*ngram);
        }
    }

    fn train_keep(&mut self, ngrams: &[Ngram]) {
        for ngram in ngrams {
            self.keep.inc(*ngram);
        }
    }

    fn predict_delete(&mut self, ngrams: &[Ngram]) -> f64 {
        let mut log_p = 0.0;
        for ngram in ngrams {
            log_p += self.delete.log_p(ngram);
            log_p -= self.keep.log_p(ngram);
        }
        log_p
    }
}

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
enum Tokenize {
    Words,
    Chars,
}

#[derive(Parser, Debug, Clone)]
struct Args {
    #[clap(short, default_value = "3")]
    k: usize,

    #[clap(required = true)]
    paths: Vec<PathBuf>,

    #[clap(long, default_value = "delete.txt")]
    delete: PathBuf,

    #[clap(long, default_value = "keep.txt")]
    keep: PathBuf,

    #[clap(long, default_value = "info")]
    log_level: String,

    #[clap(short, long)]
    fullscreen: bool,

    #[clap(long, default_value = "words")]
    tokenize: Tokenize,

    #[arg(
        long,
        value_delimiter = ',',
        default_value = "avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4"
    )]
    video_exts: Vec<String>,
}

#[derive(Debug)]
struct State {
    path: PathBuf,
    contents: Vec<String>,
}

impl State {
    fn new(path: &Path) -> State {
        State {
            path: path.to_owned(),
            contents: Vec::new(),
        }
    }

    fn load(&mut self) -> io::Result<()> {
        match File::open(&self.path) {
            Ok(file) => {
                let reader = io::BufReader::new(file);
                for line in reader.lines().map_while(Result::ok) {
                    self.contents.push(line);
                }
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn from(path: &Path) -> io::Result<State> {
        let mut state = State::new(path);
        state.load()?;
        Ok(state)
    }

    fn update(&mut self, line: &str) -> io::Result<()> {
        self.contents.push(line.to_owned());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    fn iter(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.contents.iter().map(PathBuf::from)
    }
}

fn walk(roots: &Vec<PathBuf>, video_exts: &Vec<String>) -> HashSet<PathBuf> {
    let mut exts: HashSet<OsString> = HashSet::new();
    for e in video_exts {
        let mut e = OsString::from(e);
        e.make_ascii_lowercase();
        exts.insert(e);
    }

    let mut ret = HashSet::new();
    for root in roots {
        for e in WalkDir::new(root).sort_by_file_name() {
            let e = e.unwrap();
            if !e.file_type().is_file() {
                continue;
            }

            let path = e.path();
            match path.extension() {
                Some(ext) => {
                    if !exts.contains(ext) {
                        continue;
                    }
                }
                None => continue,
            }

            let canon = std::fs::canonicalize(path).unwrap();
            ret.insert(canon);
        }
    }

    ret
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Status {
    state: String,
    information: Option<Information>,
    position: f64,
    length: f64,
}

impl Status {
    fn file_name(&self) -> Option<String> {
        if let Some(i) = &self.information {
            Some(i.category.meta.filename.clone())
        } else {
            None
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Information {
    category: Category,
}

#[derive(Debug, Deserialize)]
pub struct Category {
    meta: Meta,
}

#[derive(Debug, Deserialize)]
pub struct Meta {
    filename: String,
}

struct VLCProcessHandle {
    handle: Option<Child>,
}

impl VLCProcessHandle {
    pub fn new(args: &Args, path: &Path) -> Self {
        let mut command = Command::new("vlc");
        command
            .args(&[
                "-I",
                "http",
                "--no-random",
                "--no-loop",
                "--repeat",
                "--no-play-and-exit",
                "--http-host",
                "localhost",
                "--http-port",
                "9090",
                "--http-password",
                "password",
            ])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if args.fullscreen {
            command.arg("--fullscreen");
        }

        debug!("Spawn {:?}", command);

        let child = command.spawn().expect("Failed to start VLC process");

        VLCProcessHandle {
            handle: Some(child),
        }
    }

    pub fn status(&self) -> Result<Status, Error> {
        let url = "http://:password@localhost:9090/requests/status.json";
        let response = reqwest::blocking::get(url)?;
        let text = response.text()?;
        //debug!("Response: {}", text);
        Ok(serde_json::from_str(&text)?)
    }

    pub fn wait_for_status(&self) -> Result<Status, Error> {
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(status) = self.status() {
                return Ok(status);
            }
        }
        Err(Error::Timeout)
    }
}

impl Drop for VLCProcessHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.handle.take() {
            let kill_result = child.kill();
            debug!("kill {:?}", kill_result);
            let wait_result = child.wait();
            debug!("wait {:?}", wait_result);
        }
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &args.log_level);
    }
    env_logger::init();

    info!("{:#?}", args);

    let mut files = walk(&args.paths, &args.video_exts);
    assert!(!files.is_empty());

    let tokenizer = Tokenizer::new(args.k, args.tokenize, &files);
    let mut classifier = NaiveBayesClassifier::new(&tokenizer);

    let mut delete = State::from(&args.delete)?;
    for path in delete.iter() {
        let ngrams = tokenizer.ngrams_cached(&path);
        classifier.train_delete(&ngrams);
        files.remove(&path);
    }

    let mut keep = State::from(&args.keep)?;
    for path in keep.iter() {
        let ngrams = tokenizer.ngrams_cached(&path);
        classifier.train_keep(&ngrams);
        files.remove(&path);
    }

    let mut files_vec: Vec<(PathBuf, Vec<Ngram>, f64)> = Vec::new();
    for path in files.into_iter() {
        let ngrams = tokenizer.ngrams_cached(&path);
        files_vec.push((path, ngrams, 1.0));
    }

    while !files_vec.is_empty() {
        for (_, ngrams, ref mut score) in files_vec.iter_mut() {
            *score = classifier.predict_delete(ngrams);
        }

        files_vec.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

        //println!();
        //for (i, (path, _, score)) in files_vec.iter().rev().enumerate().take(10) {
        //    println!(
        //        "{} {:<2} {:>6.2} {:?} ",
        //        if i == 0 { ">>" } else { "  " },
        //        i + 1,
        //        score,
        //        path
        //    );
        //}

        let (path, ngrams, _) = files_vec.pop().unwrap();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let path_str = path.to_string_lossy().to_string();
        info!("{:?}", path);

        let vlc = VLCProcessHandle::new(&args, &path);
        match vlc.wait_for_status() {
            Ok(status) => {
                let found_file_name = status.file_name();
                if Some(&file_name) != found_file_name.as_ref() {
                    error!(
                        "Filename mismatch {:?} {:?}, skipping",
                        file_name, found_file_name
                    );
                    continue;
                }
            }
            Err(e) => {
                error!("Vlc startup error {:?}", e);
                continue;
            }
        }

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

            match status.state.as_str() {
                "stopped" => {
                    delete.update(&path_str)?;
                    classifier.train_delete(&ngrams);
                    info!("{:?} (DELETE)", path);
                    break;
                }
                "paused" => {
                    keep.update(&path_str)?;
                    classifier.train_keep(&ngrams);
                    info!("{:?} (KEEP)", path);
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}
