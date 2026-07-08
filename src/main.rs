mod app;
mod bloom;
mod cache;
mod classifier;
mod ffprobe;
mod logging;
mod ngrams;
mod normalize;
mod path;
mod playlist;
mod tokenize;
mod tokens;
mod vlc;
mod walk;

use crate::app::App;
use crate::path::PathDisplayContext;
use crate::playlist::{M3uPlaylist, Playlist};
use clap::{Parser, Subcommand};
use log::*;
use std::path::{Path, PathBuf};

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

    #[error("ffprobe failed for {path}: {reason}")]
    ProbeFailed { path: String, reason: String },

    #[error("Cache error: {0}")]
    Cache(String),
}

#[derive(Parser, Debug, Clone)]
#[command(name = "classi-cine")]
struct Args {
    #[command(subcommand)]
    command: Command,

    #[clap(long, default_value = "info")]
    log_level: String,

    /// Write log output to this file. When set, logs always go to the file,
    /// even while the interactive TUI is running (which suppresses stderr logs).
    #[clap(long)]
    log_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug, Clone)]
enum Command {
    /// Build playlist through interactive classification
    Build(BuildArgs),
    /// Score files using trained classifiers without interactive classification
    Score(ScoreArgs),
    /// List positively classified files
    ListPositive(ListArgs),
    /// List negatively classified files
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
struct CommonArgs {
    /// M3U playlist file
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
    /// Maximum contiguous window size for ngram features. Set to 0 to disable
    /// windows and rely solely on --combinations.
    #[clap(long, default_value_t = 5)]
    windows: usize,
    /// Generate orderless combinations of up to k tokens (default pairs) as
    /// ngram features. Independent of --windows, so --windows=0 leaves only
    /// combinations; set --combinations=0 to disable them entirely.
    #[clap(long, default_value_t = 2)]
    combinations: usize,
    #[command(flatten)]
    file_size: FileSizeArgs,
    #[command(flatten)]
    dir_size: DirSizeArgs,
    #[command(flatten)]
    file_age: FileAgeArgs,
    /// Cache TTL in days for the ffprobe feature cache. Entries whose key is
    /// not seen among the collected files for this long are expired. 0
    /// disables expiry entirely (useful for cold, stable libraries). To force
    /// expire everything, delete the cache directory.
    #[clap(long, default_value_t = 30)]
    cache_ttl_days: u32,
}

#[derive(Parser, Debug, Clone)]
struct BuildArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[command(flatten)]
    vlc: VlcArgs,
    /// Iterate top-scored entries and select the first where rand() <= p
    #[clap(long, value_parser = clap::value_parser!(f64))]
    selection_p: Option<f64>,
}

#[derive(Parser, Debug, Clone)]
struct ScoreArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// Include already classified files in the score listing
    #[clap(long)]
    include_classified: bool,
    /// Skip header output for machine-readable format
    #[clap(long)]
    no_header: bool,
    /// Include file size in bytes in output
    #[clap(long)]
    include_size: bool,
    /// Output results in JSON format
    #[clap(long)]
    json: bool,
    /// Reverse output order (lowest scores first)
    #[clap(long)]
    reverse: bool,
    /// Group results by directory and aggregate scores
    #[clap(long)]
    by_dir: bool,
    /// Display absolute paths instead of relative to current directory
    #[clap(long)]
    absolute: bool,
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
    /// Display absolute paths instead of relative to current directory
    #[clap(long)]
    absolute: bool,
}

fn move_playlist(original_path: &Path, new_path: &Path) -> Result<(), Error> {
    // Load the original playlist
    let original_playlist = M3uPlaylist::open(original_path)?;

    // Create a new playlist at the target location
    let mut new_playlist = M3uPlaylist::open(new_path)?;

    info!(
        "Moving playlist from {} to {}",
        original_playlist.path().display(),
        new_playlist.path().display()
    );

    // Process all entries in original order, preserving their original
    // `added` timestamps and scores.
    for entry in original_playlist.entries() {
        let abs = entry.abs_path(original_playlist.root());
        new_playlist.add_entry(&abs, entry.added, entry.score(), &entry.features)?;
        debug!(
            "Moved {} entry: {}",
            if entry.is_positive() {
                "positive"
            } else {
                "negative"
            },
            abs.display()
        );
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

fn list_entries(playlist_path: &Path, filter: ListFilter, absolute: bool) -> Result<(), Error> {
    let playlist = M3uPlaylist::open(playlist_path)?;

    let context = PathDisplayContext::score_list_context(absolute);

    for entry in playlist.entries() {
        let matches = match &filter {
            ListFilter::Positive => entry.is_positive(),
            ListFilter::Negative => entry.is_negative(),
        };
        if matches {
            let display_path = entry.abs_path(playlist.root()).to_string(&context);
            println!("{}", display_path);
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    logging::init(&args.log_level, args.log_file.as_deref())?;

    match args.command {
        Command::Build(ref build_args) => {
            // Validate selection probability range
            if let Some(p) = build_args.selection_p
                && !(0.0..=1.0).contains(&p)
            {
                return Err(Error::PlaylistError(
                    "--selection-p must be in [0.0, 1.0]".to_string(),
                ));
            }

            let playlist = M3uPlaylist::open(&build_args.common.playlist)?;
            let mut app = App::new(build_args.clone(), playlist);
            app.run_build()?;
        }
        Command::Score(ref score_args) => {
            let playlist = M3uPlaylist::open(&score_args.common.playlist)?;
            let mut app = App::new_for_scoring(score_args.clone(), playlist);
            app.run_score()?;
        }
        Command::ListPositive(list_args) => {
            list_entries(
                &list_args.playlist,
                ListFilter::Positive,
                list_args.absolute,
            )?;
        }
        Command::ListNegative(list_args) => {
            list_entries(
                &list_args.playlist,
                ListFilter::Negative,
                list_args.absolute,
            )?;
        }
        Command::Move(move_args) => {
            move_playlist(&move_args.original, &move_args.new)?;
        }
    }
    Ok(())
}
