mod tokenizer;
use tokenizer::{Ngram, Tokenize, Tokenizer};

mod walk;
use walk::Walk;

mod vlc;
use vlc::VLCProcessHandle;

mod classifier;
use classifier::NaiveBayesClassifier;

use clap::Parser;
use humansize::{format_size, BINARY};
use log::*;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use textplots::{Chart, Plot, Shape};

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

fn round(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
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

    #[clap(long)]
    file_size_log_base: Option<f64>,

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

#[derive(Debug, Default)]
struct FileState {
    path: PathBuf,
    // Classifier state.
    ngrams: Vec<Ngram>,
    classifier_score: f64,
    // File size state.
    file_size: u64,
    file_size_score: f64,

    score: f64,
}

impl FileState {
    fn new(
        path: PathBuf,
        ngrams: Vec<Ngram>,
        file_size: u64,
        file_size_log_base: Option<f64>,
    ) -> Self {
        let file_size_score = if let Some(base) = file_size_log_base {
            ((file_size + 1) as f64).log(base)
        } else {
            0.0
        };
        Self {
            path,
            ngrams,
            file_size,
            file_size_score,
            classifier_score: 0.0,
            score: 0.0,
        }
    }

    fn update(&mut self, classifier: &NaiveBayesClassifier) {
        self.classifier_score = classifier.predict_delete(&self.ngrams);
        self.score = self.file_size_score + self.classifier_score;
    }

    fn debug(&self, tokenizer: &Tokenizer, classifier: &NaiveBayesClassifier) {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Current<'a> {
            path: &'a Path,
            size: String,
            classifier_score: f64,
            file_size_score: f64,
            ngrams: Vec<(f64, String)>,
        }
        let debug = Current {
            path: &self.path,
            size: format_size(self.file_size, BINARY),
            classifier_score: round(self.classifier_score),
            file_size_score: round(self.file_size_score),
            ngrams: classifier.debug_delete(tokenizer, &self.ngrams),
        };
        println!("{:?}", debug);
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", &args.log_level);
    }
    env_logger::init();

    info!("{:#?}", args);

    let walk = Walk::new(&args.video_exts);
    for path in &args.paths {
        walk.root(path);
    }

    let mut files = walk.collect();
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

    let mut files_vec: Vec<FileState> = Vec::new();
    for (path, size) in files.into_iter() {
        let ngrams = tokenizer.ngrams_cached(&path);
        files_vec.push(FileState::new(path, ngrams, size, args.file_size_log_base));
    }

    while !files_vec.is_empty() {
        for file in files_vec.iter_mut() {
            file.update(&classifier);
        }

        files_vec.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

        println!();
        {
            let mut xmin = 0.0;
            let mut xmax = 0.0;
            let mut ymin = 0.0;
            let mut ymax = 0.0;
            let mut points = Vec::new();
            for (i, file) in files_vec.iter().enumerate() {
                let (x, y) = (i as f32, file.file_size_score as f32);
                xmin = f32::min(xmin, x);
                xmax = f32::max(xmax, x);
                ymin = f32::min(ymin, y);
                ymax = f32::max(ymax, y);
                points.push((x, y));
            }
            println!("File size scores");
            Chart::new_with_y_range(300, 80, xmin, xmax, ymin, ymax)
                .lineplot(&Shape::Points(&points))
                .nice();
        }

        {
            let mut xmin = 0.0;
            let mut xmax = 0.0;
            let mut ymin = 0.0;
            let mut ymax = 0.0;
            let mut points = Vec::new();
            println!("Classifier scores");
            for (i, file) in files_vec.iter().enumerate() {
                let (x, y) = (i as f32, file.classifier_score as f32);
                xmin = f32::min(xmin, x);
                xmax = f32::max(xmax, x);
                ymin = f32::min(ymin, y);
                ymax = f32::max(ymax, y);
                points.push((x, y));
            }
            Chart::new_with_y_range(300, 80, xmin, xmax, ymin, ymax)
                .lineplot(&Shape::Points(&points))
                .nice();
        }

        let file_state = files_vec.pop().unwrap();

        file_state.debug(&tokenizer, &classifier);

        let file_name = file_state
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let path_str = file_state.path.to_string_lossy().to_string();

        let vlc = VLCProcessHandle::new(&args, &file_state.path);
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

            match status.state() {
                "stopped" => {
                    delete.update(&path_str)?;
                    classifier.train_delete(&file_state.ngrams);
                    info!("{:?} (DELETE)", path_str);
                    break;
                }
                "paused" => {
                    keep.update(&path_str)?;
                    classifier.train_keep(&file_state.ngrams);
                    info!("{:?} (KEEP)", path_str);
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}
