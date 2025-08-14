use crate::Error;
use crate::path::{AbsPath, PathDisplayContext};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const M3U_HEADER: &str = "#EXTM3U";
const NEGATIVE_PREFIX: &str = "#NEGATIVE:";

/// Represents a playlist entry type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaylistEntry {
    Positive(AbsPath),
    Negative(AbsPath),
}

impl PlaylistEntry {
    /// Returns the path regardless of entry type
    pub fn path(&self) -> &AbsPath {
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
    path: AbsPath,
    root: AbsPath,
    entries: Vec<PlaylistEntry>, // Single vector for all entries in order
}

impl M3uPlaylist {
    pub fn path(&self) -> &Path {
        self.path.abs_path()
    }

    pub fn root(&self) -> &Path {
        self.root.abs_path()
    }

    pub fn display_path(&self, abs_path: &AbsPath, context: &PathDisplayContext) -> String {
        abs_path.to_string(context)
    }

    pub fn open(path: &Path) -> Result<Self, Error> {
        // Create an absolute path to the playlist, normalizing it.
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let abs_path_buf = crate::normalize::normalize_path(&path);
        let abs_path = AbsPath::from_abs_path(&abs_path_buf);
        let root = AbsPath::from_abs_path(abs_path_buf.parent().unwrap());

        let mut playlist = Self {
            path: abs_path,
            root,
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
                        let abs_path = playlist.root().join(&rel_path);
                        playlist
                            .entries
                            .push(PlaylistEntry::Negative(AbsPath::from_abs_path(&abs_path)));
                    }
                } else if !line.starts_with('#') {
                    // Positive classification (regular entry)
                    let rel_path = PathBuf::from(line.trim());
                    let abs_path = playlist.root().join(&rel_path);
                    playlist
                        .entries
                        .push(PlaylistEntry::Positive(AbsPath::from_abs_path(&abs_path)));
                }
            }
        }

        Ok(playlist)
    }
}

impl Playlist for M3uPlaylist {
    fn add_positive(&mut self, abs_path: &Path) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let context = PathDisplayContext::RelativeTo(self.root.abs_path().to_path_buf());
        let rel_path = abs_path.to_string(&context);
        self.entries.push(PlaylistEntry::Positive(abs_path));
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}", rel_path)?;
        Ok(())
    }

    fn add_negative(&mut self, abs_path: &Path) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let context = PathDisplayContext::RelativeTo(self.root.abs_path().to_path_buf());
        let rel_path = abs_path.to_string(&context);
        self.entries.push(PlaylistEntry::Negative(abs_path));
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "{}{}", NEGATIVE_PREFIX, rel_path)?;
        Ok(())
    }

    fn entries(&self) -> &[PlaylistEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_playlist_path_handling() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let playlist_path = temp_dir.path().join("playlist.m3u");

        // 1. Test creating a new playlist and checking its root
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        let expected_root = crate::normalize::normalize_path(temp_dir.path());
        assert_eq!(playlist.root(), expected_root);
        assert!(playlist.path().is_absolute());

        // 2. Test adding a file and checking the relative path
        let track1_path = music_dir.join("track1.mp3");
        playlist.add_positive(&track1_path)?;

        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("music/track1.mp3"));

        // 3. Test re-opening and loading entries
        let playlist = M3uPlaylist::open(&playlist_path)?;
        let expected_abs_path = music_dir.join("track1.mp3");
        let expected_abs_path = crate::normalize::normalize_path(&expected_abs_path);
        assert_eq!(playlist.entries().len(), 1);
        match &playlist.entries()[0] {
            PlaylistEntry::Positive(path) => {
                assert_eq!(path.abs_path(), expected_abs_path);
            }
            _ => panic!("Expected positive entry"),
        }

        // 4. Test adding a negative entry
        let track2_path = music_dir.join("track2.mp3");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.add_negative(&track2_path)?;
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("#NEGATIVE:music/track2.mp3"));

        // 5. Test opening from a relative path
        std::env::set_current_dir(temp_dir.path())?;
        let relative_playlist_path = Path::new("playlist.m3u");
        let playlist = M3uPlaylist::open(relative_playlist_path)?;
        assert_eq!(playlist.root(), expected_root);

        Ok(())
    }
}
