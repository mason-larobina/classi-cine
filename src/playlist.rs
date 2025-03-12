use crate::Error;
use log::*;
use pathdiff::diff_paths;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const M3U_HEADER: &str = "#EXTM3U";
const NEGATIVE_PREFIX: &str = "#NEGATIVE:";

/// Represents a playlist entry type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistEntry {
    Positive(PathBuf),
    Negative(PathBuf),
}

impl PlaylistEntry {
    /// Returns the path regardless of entry type
    pub fn path(&self) -> &PathBuf {
        match self {
            PlaylistEntry::Positive(path) => path,
            PlaylistEntry::Negative(path) => path,
        }
    }
}

/// Trait for types that can load/save playlist classifications
pub trait Playlist {
    /// Add a positive classification
    fn add_positive(&mut self, path: &Path) -> Result<(), Error>;

    /// Add a negative classification
    fn add_negative(&mut self, path: &Path) -> Result<(), Error>;

    /// Get all entries in order
    fn entries(&self) -> &[PlaylistEntry];
}

/// M3U playlist implementation that tracks positive/negative classifications
pub struct M3uPlaylist {
    path: PathBuf,
    entries: Vec<PlaylistEntry>, // Single vector for all entries in order
}

impl M3uPlaylist {
    pub fn path(&self) -> &Path {
        &self.path
    }

    // Helper method to convert absolute paths to relative (relative to playlist directory)
    pub fn to_relative_path(&self, path: &Path) -> PathBuf {
        let parent = self.path.parent().unwrap();
        diff_paths(path, parent).unwrap_or_else(|| path.to_path_buf())
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        // Make sure we have an absolute path for the playlist file
        let path = std::path::absolute(path).unwrap();

        let mut playlist = Self {
            path,
            entries: Vec::new(),
        };

        if !playlist.path.exists() {
            // Create new file with M3U header
            let mut file = File::create(&playlist.path)?;
            writeln!(file, "{}", M3U_HEADER)?;
        } else {
            // Load and verify existing file
            let file = File::open(&playlist.path)?;
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            // Verify M3U header in existing file
            let first_line = lines
                .next()
                .ok_or_else(|| Error::PlaylistError("Empty playlist file".to_string()))??;

            if first_line.trim() != M3U_HEADER {
                return Err(Error::PlaylistError(
                    "Existing playlist file missing M3U header".to_string(),
                ));
            }

            // Process remaining lines
            for line in lines {
                let line = line?;
                if line.starts_with(NEGATIVE_PREFIX) {
                    // Negative classification (commented out)
                    if let Some(path_str) = line.strip_prefix(NEGATIVE_PREFIX) {
                        let rel_path = PathBuf::from(path_str.trim());
                        playlist.entries.push(PlaylistEntry::Negative(rel_path));
                    }
                } else if !line.starts_with('#') {
                    // Positive classification (regular entry)
                    let rel_path = PathBuf::from(line.trim());
                    playlist.entries.push(PlaylistEntry::Positive(rel_path));
                }
            }
        }

        Ok(playlist)
    }
}

impl Playlist for M3uPlaylist {
    fn add_positive(&mut self, path: &Path) -> Result<(), Error> {
        let relative_path = self.to_relative_path(path);
        self.entries
            .push(PlaylistEntry::Positive(relative_path.clone()));
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}", relative_path.display())?;
        Ok(())
    }

    fn add_negative(&mut self, path: &Path) -> Result<(), Error> {
        let relative_path = self.to_relative_path(path);
        self.entries
            .push(PlaylistEntry::Negative(relative_path.clone()));
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}{}", NEGATIVE_PREFIX, relative_path.display())?;
        Ok(())
    }

    fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }
}
