use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const M3U_HEADER: &str = "#EXTM3U";
const NEGATIVE_PREFIX: &str = "#NEGATIVE:";

/// Trait for types that can load/save playlist classifications
pub trait Playlist {
    /// Add a positive classification
    fn add_positive(&mut self, path: &Path) -> io::Result<()>;

    /// Add a negative classification
    fn add_negative(&mut self, path: &Path) -> io::Result<()>;

    /// Get positive classifications
    fn positives(&self) -> &HashSet<PathBuf>;

    /// Get negative classifications
    fn negatives(&self) -> &HashSet<PathBuf>;
}

/// M3U playlist implementation that tracks positive/negative classifications
pub struct M3uPlaylist {
    path: PathBuf,
    positives: HashSet<PathBuf>,
    negatives: HashSet<PathBuf>,
}

impl M3uPlaylist {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut playlist = Self {
            path: path.to_path_buf(),
            positives: HashSet::new(),
            negatives: HashSet::new(),
        };

        if !path.exists() {
            // Create new file with M3U header
            let mut file = File::create(&path)?;
            writeln!(file, "{}", M3U_HEADER)?;
        } else {
            // Load and verify existing file
            let file = File::open(&path)?;
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            // Verify M3U header in existing file
            let first_line = lines.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Empty playlist file")
            })??;
            if first_line.trim() != M3U_HEADER {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Existing playlist file missing M3U header",
                ));
            }

            // Process remaining lines
            for line in lines {
                let line = line?;
                if line.starts_with(NEGATIVE_PREFIX) {
                    // Negative classification (commented out)
                    if let Some(path) = line.strip_prefix(NEGATIVE_PREFIX) {
                        playlist.negatives.insert(PathBuf::from(path.trim()));
                    }
                } else if !line.starts_with('#') {
                    // Positive classification (regular entry)
                    playlist.positives.insert(PathBuf::from(line.trim()));
                }
            }
        }

        Ok(playlist)
    }
}

impl Playlist for M3uPlaylist {
    fn add_positive(&mut self, path: &Path) -> io::Result<()> {
        self.positives.insert(path.to_path_buf());

        let mut file = OpenOptions::new().append(true).open(&self.path)?;

        writeln!(file, "{}", path.display())?;
        Ok(())
    }

    fn add_negative(&mut self, path: &Path) -> io::Result<()> {
        self.negatives.insert(path.to_path_buf());

        let mut file = OpenOptions::new().append(true).open(&self.path)?;

        writeln!(file, "{}{}", NEGATIVE_PREFIX, path.display())?;
        Ok(())
    }

    fn positives(&self) -> &HashSet<PathBuf> {
        &self.positives
    }

    fn negatives(&self) -> &HashSet<PathBuf> {
        &self.negatives
    }
}
