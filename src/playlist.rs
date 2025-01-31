use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::collections::HashSet;

const M3U_HEADER: &str = "#EXTM3U";
const NEGATIVE_PREFIX: &str = "#NEGATIVE:";

/// Trait for types that can load/save playlist classifications
pub trait Playlist {
    /// Load positive and negative classifications from a playlist file
    fn load(&mut self, path: &Path) -> io::Result<()>;
    
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
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            positives: HashSet::new(),
            negatives: HashSet::new(),
        }
    }
}

impl Playlist for M3uPlaylist {
    fn load(&mut self, path: &Path) -> io::Result<()> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        // Verify M3U header
        if let Some(Ok(first_line)) = lines.next() {
            if first_line != M3U_HEADER {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing M3U header",
                ));
            }
        }

        // Process remaining lines
        for line in lines {
            let line = line?;
            if line.starts_with(NEGATIVE_PREFIX) {
                // Negative classification (commented out)
                if let Some(path) = line.strip_prefix(NEGATIVE_PREFIX) {
                    self.negatives.insert(PathBuf::from(path.trim()));
                }
            } else if !line.starts_with('#') {
                // Positive classification (regular entry)
                self.positives.insert(PathBuf::from(line.trim()));
            }
        }

        Ok(())
    }

    fn add_positive(&mut self, path: &Path) -> io::Result<()> {
        self.positives.insert(path.to_path_buf());
        
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        // Write header if file is empty
        if file.metadata()?.len() == 0 {
            writeln!(file, "{}", M3U_HEADER)?;
        }

        writeln!(file, "{}", path.display())?;
        Ok(())
    }

    fn add_negative(&mut self, path: &Path) -> io::Result<()> {
        self.negatives.insert(path.to_path_buf());
        
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        // Write header if file is empty
        if file.metadata()?.len() == 0 {
            writeln!(file, "{}", M3U_HEADER)?;
        }

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
