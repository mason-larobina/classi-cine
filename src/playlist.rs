use crate::Error;
use crate::path::{AbsPath, PathDisplayContext};
use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const M3U_HEADER: &str = "#EXTM3U";
/// Prefix for the structured metadata comment line, e.g. `#{...}`.
const META_PREFIX: &str = "#{";

/// Positive classification score.
pub const SCORE_POSITIVE: i32 = 1;
/// Negative classification score.
pub const SCORE_NEGATIVE: i32 = -1;

/// Structured metadata for a playlist entry, serialized as JSON inside an M3U
/// comment line of the form `#{<json>}`.
///
/// Fields use short serialized keys (aliased to their long form on
/// deserialization for backward compatibility) to minimize per-entry overhead:
/// - `file` (`f`): path to the media file, relative to the playlist root.
/// - `added` (`a`): unix timestamp (seconds) marking when the classification
///   was recorded.
/// - `score` (`s`): classification score (+1 positive, -1 negative).
///
/// Unknown fields are tolerated on deserialization so that future extensions
/// (e.g. ffprobe features) can be added without breaking older readers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EntryMeta {
    #[serde(rename = "f", alias = "file")]
    pub file: String,
    #[serde(rename = "a", alias = "added")]
    pub added: i64,
    #[serde(rename = "s", alias = "score")]
    pub score: i32,
}

/// A single playlist entry carrying its absolute path and classification metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistEntry {
    path: AbsPath,
    meta: EntryMeta,
}

impl PlaylistEntry {
    pub fn new(path: AbsPath, meta: EntryMeta) -> Self {
        Self { path, meta }
    }

    /// Returns the absolute path regardless of entry type.
    pub fn path(&self) -> &AbsPath {
        &self.path
    }

    /// Returns the classification metadata.
    pub fn meta(&self) -> &EntryMeta {
        &self.meta
    }

    /// Returns the classification score (+1 positive, -1 negative).
    pub fn score(&self) -> i32 {
        self.meta.score
    }

    /// Returns true if this is a positive classification.
    pub fn is_positive(&self) -> bool {
        self.meta.score > 0
    }

    /// Returns true if this is a negative classification.
    pub fn is_negative(&self) -> bool {
        self.meta.score < 0
    }
}

/// Trait for types that can load/save playlist classifications
pub trait Playlist {
    /// Add a positive classification (records the current time as `added`).
    fn add_positive(&mut self, path: &Path) -> Result<(), Error>;

    /// Add a negative classification (records the current time as `added`).
    fn add_negative(&mut self, path: &Path) -> Result<(), Error>;

    /// Append an existing entry, preserving its original `added` time and score.
    /// The stored `file` path is recomputed relative to this playlist's root.
    fn add_entry(&mut self, entry: &PlaylistEntry) -> Result<(), Error>;

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
        &self.path
    }

    pub fn root(&self) -> &AbsPath {
        &self.root
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
        let abs_path = AbsPath::from_abs_path(&path);
        let root = AbsPath::from_abs_path(path.parent().unwrap());

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
            let mut lines = reader.lines().peekable();

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
            while let Some(line) = lines.next() {
                let line = line?;
                let trimmed = line.trim();

                if let Some(json_str) = trimmed.strip_prefix(META_PREFIX) {
                    // Structured metadata line: `#{<json>}`. The `#` was consumed
                    // by strip_prefix via META_PREFIX ("#{"), leaving the JSON
                    // object body which still begins with `{`.
                    let json_str = format!("{{{}", json_str);
                    let meta: EntryMeta = serde_json::from_str(&json_str)?;

                    let rel_path = PathBuf::from(&meta.file);
                    let abs_entry = playlist.root().join(&rel_path);
                    let abs_entry = AbsPath::from_abs_path(&abs_entry);

                    // Positive entries are followed by a bare filename line (so
                    // other M3U-aware apps still pick them up). Consume it here.
                    if meta.score > 0
                        && let Some(Ok(next)) = lines.peek()
                    {
                        let next_trimmed = next.trim();
                        if !next_trimmed.is_empty() && !next_trimmed.starts_with('#') {
                            let _ = lines.next();
                        }
                    }

                    playlist.entries.push(PlaylistEntry::new(abs_entry, meta));
                }
                // Any other line (bare filename, other comments) is ignored: the
                // new format stores everything needed in the `#{...}` meta line.
            }
        }

        Ok(playlist)
    }

    /// Write an entry to disk and record it. The `file` field is recomputed
    /// relative to this playlist's root from the supplied absolute path.
    fn append_entry(&mut self, abs_path: &AbsPath, score: i32, added: i64) -> Result<(), Error> {
        let context = PathDisplayContext::RelativeTo(self.root.to_path_buf());
        let rel_path = abs_path.to_string(&context);
        let meta = EntryMeta {
            file: rel_path.clone(),
            added,
            score,
        };
        let json = serde_json::to_string(&meta)?;
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "#{}", json)?;
        // Positive entries are duplicated as a bare filename line so that the
        // playlist remains usable by other M3U-aware applications.
        if score > 0 {
            writeln!(file, "{}", rel_path)?;
        }
        self.entries
            .push(PlaylistEntry::new(abs_path.clone(), meta));
        Ok(())
    }
}

impl Playlist for M3uPlaylist {
    fn add_positive(&mut self, abs_path: &Path) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let added = Utc::now().timestamp();
        self.append_entry(&abs_path, SCORE_POSITIVE, added)
    }

    fn add_negative(&mut self, abs_path: &Path) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let added = Utc::now().timestamp();
        self.append_entry(&abs_path, SCORE_NEGATIVE, added)
    }

    fn add_entry(&mut self, entry: &PlaylistEntry) -> Result<(), Error> {
        // Preserve the original `added` timestamp and score, but recompute the
        // `file` path relative to this playlist's root.
        self.append_entry(entry.path(), entry.score(), entry.meta().added)
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
        let expected_root = crate::path::normalize_path(temp_dir.path());
        assert_eq!(&**playlist.root(), expected_root);
        assert!(playlist.path().is_absolute());

        // 2. Test adding a positive entry: should emit a meta line and a
        // duplicated filename line.
        let track1_path = music_dir.join("track1.mp3");
        playlist.add_positive(&track1_path)?;

        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("#"));
        assert!(content.contains("\"f\":\"music/track1.mp3\""));
        assert!(content.contains("\"s\":1"));
        // The filename line is duplicated for compatibility with other apps.
        assert!(content.contains("\nmusic/track1.mp3\n"));

        // 3. Test re-opening and loading entries
        let playlist = M3uPlaylist::open(&playlist_path)?;
        let expected_abs_path = music_dir.join("track1.mp3");
        assert_eq!(playlist.entries().len(), 1);
        let entry = &playlist.entries()[0];
        assert!(entry.is_positive());
        assert!(!entry.is_negative());
        assert_eq!(entry.score(), 1);
        assert_eq!(entry.path().as_ref(), expected_abs_path);

        // 4. Test adding a negative entry: meta line only, no filename line.
        let track2_path = music_dir.join("track2.mp3");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.add_negative(&track2_path)?;
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("\"f\":\"music/track2.mp3\""));
        assert!(content.contains("\"s\":-1"));
        // No bare filename line for negative entries.
        assert!(!content.contains("\nmusic/track2.mp3\n"));

        // 5. Test reopening loads both entries with correct polarities.
        let playlist = M3uPlaylist::open(&playlist_path)?;
        assert_eq!(playlist.entries().len(), 2);
        assert!(playlist.entries()[0].is_positive());
        assert!(playlist.entries()[1].is_negative());

        // 6. Test opening from a relative path
        std::env::set_current_dir(temp_dir.path())?;
        let relative_playlist_path = Path::new("playlist.m3u");
        let playlist = M3uPlaylist::open(relative_playlist_path)?;
        assert_eq!(&**playlist.root(), expected_root);

        Ok(())
    }

    #[test]
    fn test_playlist_relative_path_normalization() -> Result<(), Error> {
        let temp_dir = tempdir()?;

        // Create complex directory structure
        let music_dir = temp_dir.path().join("music");
        let subdir = music_dir.join("subdir");
        let other_dir = temp_dir.path().join("other");
        std::fs::create_dir_all(&subdir)?;
        std::fs::create_dir_all(&other_dir)?;

        // Create test files
        std::fs::write(music_dir.join("track1.mp3"), b"test")?;
        std::fs::write(subdir.join("track2.mp3"), b"test")?;
        std::fs::write(other_dir.join("track3.mp3"), b"test")?;

        let playlist_path = temp_dir.path().join("playlist.m3u");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;

        // Test 1: Path with current directory reference (./)
        let path_with_dot = music_dir.join("./track1.mp3");
        playlist.add_positive(&path_with_dot)?;

        // Test 2: Path with parent directory reference (../)
        let path_with_dotdot = subdir.join("../track1.mp3");
        playlist.add_positive(&path_with_dotdot)?;

        // Test 3: Complex path with multiple . and ..
        let complex_path = subdir.join("./../../other/../music/./track1.mp3");
        playlist.add_positive(&complex_path)?;

        // Test 4: Path that goes up and then down to other directory
        let cross_path = music_dir.join("../other/track3.mp3");
        playlist.add_positive(&cross_path)?;

        // Verify playlist content - all paths should be normalized to simple relative paths
        let content = std::fs::read_to_string(&playlist_path)?;
        println!("Playlist content:\n{}", content);

        // Should contain normalized relative paths
        assert!(content.contains("\"f\":\"music/track1.mp3\""));
        assert!(content.contains("\"f\":\"other/track3.mp3\""));

        // Should NOT contain any . or .. components
        assert!(!content.contains("./"));
        assert!(!content.contains("../"));

        // Re-open playlist and verify entries are correctly normalized
        let playlist = M3uPlaylist::open(&playlist_path)?;

        // All entries should point to the same normalized absolute paths
        let expected_track1 = music_dir.join("track1.mp3");
        let expected_track3 = other_dir.join("track3.mp3");

        let mut found_track1_count = 0;
        let mut found_track3_count = 0;

        for entry in playlist.entries() {
            let path_ref: &std::path::Path = entry.path().as_ref();
            if path_ref == expected_track1 {
                found_track1_count += 1;
            } else if path_ref == expected_track3 {
                found_track3_count += 1;
            }
            assert!(entry.is_positive());
        }

        // track1.mp3 should appear multiple times (added via different relative paths)
        assert!(
            found_track1_count > 1,
            "track1.mp3 should be found multiple times due to different relative paths resolving to same file"
        );
        assert_eq!(
            found_track3_count, 1,
            "track3.mp3 should be found exactly once"
        );

        Ok(())
    }

    #[test]
    fn test_move_preserves_added_and_score() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let original_path = temp_dir.path().join("original.m3u");
        let mut original = M3uPlaylist::open(&original_path)?;

        let track1 = music_dir.join("track1.mp3");
        let track2 = music_dir.join("track2.mp3");
        original.add_positive(&track1)?;
        original.add_negative(&track2)?;

        let original_added_pos = original.entries()[0].meta().added.clone();
        let original_added_neg = original.entries()[1].meta().added.clone();

        // Move to a new playlist (at the same root so relative paths are stable).
        let new_path = temp_dir.path().join("moved.m3u");
        let mut moved = M3uPlaylist::open(&new_path)?;
        for entry in original.entries() {
            moved.add_entry(entry)?;
        }

        assert_eq!(moved.entries().len(), 2);
        assert!(moved.entries()[0].is_positive());
        assert!(moved.entries()[1].is_negative());
        // `added` timestamps must survive the move.
        assert_eq!(moved.entries()[0].meta().added, original_added_pos);
        assert_eq!(moved.entries()[1].meta().added, original_added_neg);

        Ok(())
    }
}
