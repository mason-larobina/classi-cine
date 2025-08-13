mod app;
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

use crate::app::App;
use crate::playlist::{M3uPlaylist, Playlist, PlaylistEntry};
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
    /// Score files using trained classifiers without interactive classification
    Score(ScoreArgs),
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
    #[clap(long, default_value_t = 5)]
    windows: usize,
    #[command(flatten)]
    file_size: FileSizeArgs,
    #[command(flatten)]
    dir_size: DirSizeArgs,
    #[command(flatten)]
    file_age: FileAgeArgs,
}

#[derive(Parser, Debug, Clone)]
struct BuildArgs {
    #[command(flatten)]
    common: CommonArgs,
    #[command(flatten)]
    vlc: VlcArgs,
    /// Number of entries to classify in each batch iteration
    #[clap(long, default_value_t = 1)]
    batch: usize,
    /// Perform all steps except opening and classifying files.
    #[clap(long)]
    dry_run: bool,
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
        let abs_path = original_root.join(entry.path());
        let normalized_path = normalize::normalize_path(&abs_path);

        // Add to new playlist based on entry type
        match entry {
            PlaylistEntry::Positive(_) => {
                new_playlist.add_positive(&normalized_path)?;
                debug!("Moved positive entry: {}", normalized_path.display());
            }
            PlaylistEntry::Negative(_) => {
                new_playlist.add_negative(&normalized_path)?;
                debug!("Moved negative entry: {}", normalized_path.display());
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
    let root = playlist.root();
    for entry in playlist.entries() {
        match (&filter, entry) {
            (ListFilter::Positive, PlaylistEntry::Positive(_)) => {
                let path = entry.path();
                let abs_path = root.join(path);
                println!("{}", abs_path.display());
            }
            (ListFilter::Negative, PlaylistEntry::Negative(_)) => {
                let path = entry.path();
                let abs_path = root.join(path);
                println!("{}", abs_path.display());
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
            let playlist = M3uPlaylist::open(&build_args.common.playlist)?;
            let mut app = App::new(build_args.clone(), playlist);
            app.run()?;
        }
        Command::Score(ref score_args) => {
            let playlist = M3uPlaylist::open(&score_args.common.playlist)?;
            let mut app = App::new_for_scoring(score_args.clone(), playlist);
            app.score_files()?;
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
