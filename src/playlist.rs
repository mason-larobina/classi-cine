use crate::Error;
use crate::cache::MediaFeatures;
use crate::path::{AbsPath, PathDisplayContext};
use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

const M3U_HEADER: &str = "#EXTM3U";
/// Prefix for the structured metadata comment line, e.g. `#{...}`.
const META_PREFIX: &str = "#{";

/// Positive classification score.
pub const SCORE_POSITIVE: i32 = 1;
/// Negative classification score.
pub const SCORE_NEGATIVE: i32 = -1;

/// A single playlist entry: classification metadata that doubles as the
/// in-memory entry record.
///
/// `file` is stored relative to the playlist root on disk. The absolute path
/// is not cached; callers regenerate it on demand via [`abs_path`](Self::abs_path),
/// which joins `file` against the playlist's root.
///
/// Fields use short serialized keys (aliased to their long form on
/// deserialization for backward compatibility) to minimize per-entry overhead.
/// Unknown fields are tolerated on deserialization so that future extensions
/// can be added without breaking older readers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntryMeta {
    /// Path to the media file, relative to the playlist root (`f`).
    #[serde(rename = "f")]
    pub file: String,
    /// Unix timestamp (seconds) marking when the classification was recorded
    /// (`a`).
    #[serde(rename = "a")]
    pub added: i64,
    /// Classification score (+1 positive, -1 negative) (`s`).
    #[serde(rename = "s")]
    pub score: i32,
    /// The extracted ffprobe [`MediaFeatures`] captured at classification time,
    /// persisted verbatim (raw values; the classifier re-derives/buckets at
    /// read time). Defaults on read for entries written before this field
    /// existed (`m`).
    #[serde(rename = "m", default)]
    pub features: MediaFeatures,
}

impl EntryMeta {
    /// Returns the absolute path of this entry, derived by joining `file`
    /// against the playlist `root`. Not cached: regenerated on each call.
    pub fn abs_path(&self, root: &AbsPath) -> AbsPath {
        AbsPath::from_abs_path(&root.join(&self.file))
    }

    /// Returns the classification score (+1 positive, -1 negative).
    pub fn score(&self) -> i32 {
        self.score
    }

    /// Returns true if this is a positive classification.
    pub fn is_positive(&self) -> bool {
        self.score > 0
    }

    /// Returns true if this is a negative classification.
    pub fn is_negative(&self) -> bool {
        self.score < 0
    }
}

/// Trait for types that can load/save playlist classifications
pub trait Playlist {
    /// Add a positive classification (records the current time as `added`),
    /// persisting the file's extracted `features` alongside the result.
    fn add_positive(&mut self, path: &Path, features: &MediaFeatures) -> Result<(), Error>;

    /// Add a negative classification (records the current time as `added`),
    /// persisting the file's extracted `features` alongside the result.
    fn add_negative(&mut self, path: &Path, features: &MediaFeatures) -> Result<(), Error>;

    /// Append an existing entry, preserving its original `added` time,
    /// score, and `features`. The absolute `path` is rebased so the stored
    /// `file` path is recomputed relative to this playlist's root.
    fn add_entry(
        &mut self,
        path: &AbsPath,
        added: i64,
        score: i32,
        features: &MediaFeatures,
    ) -> Result<(), Error>;

    /// Get all entries in order
    fn entries(&self) -> &[EntryMeta];
}

/// M3U playlist implementation that tracks positive/negative classifications
pub struct M3uPlaylist {
    path: AbsPath,
    root: AbsPath,
    entries: Vec<EntryMeta>, // Single vector for all entries in order
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

                    playlist.entries.push(meta);
                }
                // Any other line (bare filename, other comments) is ignored: the
                // new format stores everything needed in the `#{...}` meta line.
            }
        }

        Ok(playlist)
    }

    /// Rewrite the playlist file from the in-memory entries, enforcing the M3U
    /// invariant:
    /// - Every entry (positive or negative) emits a `#{...}` metadata line.
    /// - A bare filename line is emitted only for positive entries whose file
    ///   currently exists on disk.
    ///
    /// The file is only touched when the rendered content differs from what is
    /// already on disk, so opening an already-consistent playlist is a no-op.
    pub fn reconcile(&self) -> Result<(), Error> {
        let context = PathDisplayContext::RelativeTo(self.root.to_path_buf());
        let mut rendered = String::new();
        rendered.push_str(M3U_HEADER);
        rendered.push('\n');
        for entry in &self.entries {
            let abs_path = entry.abs_path(&self.root);
            let rel_path = abs_path.to_string(&context);
            let mut meta = entry.clone();
            meta.file = rel_path.clone();
            let json = serde_json::to_string(&meta)?;
            rendered.push('#');
            rendered.push_str(&json);
            rendered.push('\n');
            // Bare filename lines are reserved for alive positive entries so
            // that VLC only references content that still exists.
            if entry.is_positive() && abs_path.as_ref().exists() {
                rendered.push_str(&rel_path);
                rendered.push('\n');
            }
        }

        let existing = std::fs::read_to_string(&self.path).unwrap_or_default();
        if existing != rendered {
            std::fs::write(&self.path, &rendered)?;
        }
        Ok(())
    }

    /// Write an entry to disk and record it. The `file` field is recomputed
    /// relative to this playlist's root from the supplied absolute path.
    ///
    /// This appends a new line pair; the full-file invariant (bare filename
    /// lines only for alive positive entries) is re-established on demand by
    /// [`reconcile`](Self::reconcile), invoked explicitly via the `reconcile`
    /// subcommand.
    fn append_entry(
        &mut self,
        abs_path: &AbsPath,
        score: i32,
        added: i64,
        features: &MediaFeatures,
    ) -> Result<(), Error> {
        let context = PathDisplayContext::RelativeTo(self.root.to_path_buf());
        let rel_path = abs_path.to_string(&context);
        let meta = EntryMeta {
            file: rel_path.clone(),
            added,
            score,
            features: features.clone(),
        };
        let json = serde_json::to_string(&meta)?;
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        writeln!(file, "#{}", json)?;
        // Positive entries are duplicated as a bare filename line so that the
        // playlist remains usable by other M3U-aware applications. If the file
        // does not exist yet (e.g. it hasn't been created on disk), the bare
        // line is omitted; it will be re-added by `reconcile` once the file
        // appears.
        if score > 0 && abs_path.as_ref().exists() {
            writeln!(file, "{}", rel_path)?;
        }
        self.entries.push(meta);
        Ok(())
    }
}

impl Playlist for M3uPlaylist {
    fn add_positive(&mut self, abs_path: &Path, features: &MediaFeatures) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let added = Utc::now().timestamp();
        self.append_entry(&abs_path, SCORE_POSITIVE, added, features)
    }

    fn add_negative(&mut self, abs_path: &Path, features: &MediaFeatures) -> Result<(), Error> {
        let abs_path = AbsPath::from_abs_path(abs_path);
        let added = Utc::now().timestamp();
        self.append_entry(&abs_path, SCORE_NEGATIVE, added, features)
    }

    fn add_entry(
        &mut self,
        path: &AbsPath,
        added: i64,
        score: i32,
        features: &MediaFeatures,
    ) -> Result<(), Error> {
        // Preserve the original `added` timestamp, score, and features, but
        // recompute the `file` path relative to this playlist's root.
        self.append_entry(path, score, added, features)
    }

    fn entries(&self) -> &[EntryMeta] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A deterministic non-default `MediaFeatures` for tests, so persisted
    /// entries carry a recognizable payload and round-trip assertions are
    /// meaningful (rather than the all-zero `Default`).
    fn test_features() -> MediaFeatures {
        MediaFeatures {
            width: 1280,
            height: 720,
            file_size: 1234,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_secs: 100.5,
            fps: Some(24.0),
        }
    }

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
        // duplicated filename line. Create the file so the bare-line invariant
        // (positive && exists) is satisfied.
        let track1_path = music_dir.join("track1.mp3");
        std::fs::write(&track1_path, b"test")?;
        playlist.add_positive(&track1_path, &test_features())?;

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
        assert_eq!(entry.abs_path(playlist.root()).as_ref(), expected_abs_path);

        // 4. Test adding a negative entry: meta line only, no filename line.
        let track2_path = music_dir.join("track2.mp3");
        std::fs::write(&track2_path, b"test")?;
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.add_negative(&track2_path, &test_features())?;
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
        playlist.add_positive(&path_with_dot, &test_features())?;

        // Test 2: Path with parent directory reference (../)
        let path_with_dotdot = subdir.join("../track1.mp3");
        playlist.add_positive(&path_with_dotdot, &test_features())?;

        // Test 3: Complex path with multiple . and ..
        let complex_path = subdir.join("./../../other/../music/./track1.mp3");
        playlist.add_positive(&complex_path, &test_features())?;

        // Test 4: Path that goes up and then down to other directory
        let cross_path = music_dir.join("../other/track3.mp3");
        playlist.add_positive(&cross_path, &test_features())?;

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
            let abs = entry.abs_path(playlist.root());
            let path_ref: &std::path::Path = abs.as_ref();
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
        std::fs::write(&track1, b"test")?;
        std::fs::write(&track2, b"test")?;
        original.add_positive(&track1, &test_features())?;
        original.add_negative(&track2, &test_features())?;

        let original_added_pos = original.entries()[0].added;
        let original_added_neg = original.entries()[1].added;

        // Move to a new playlist (at the same root so relative paths are stable).
        let new_path = temp_dir.path().join("moved.m3u");
        let mut moved = M3uPlaylist::open(&new_path)?;
        for entry in original.entries() {
            let abs = entry.abs_path(original.root());
            moved.add_entry(&abs, entry.added, entry.score(), &entry.features)?;
        }

        assert_eq!(moved.entries().len(), 2);
        assert!(moved.entries()[0].is_positive());
        assert!(moved.entries()[1].is_negative());
        // `added` timestamps must survive the move.
        assert_eq!(moved.entries()[0].added, original_added_pos);
        assert_eq!(moved.entries()[1].added, original_added_neg);

        Ok(())
    }

    /// The bare filename line is reserved for positive entries whose file
    /// still exists on disk. On open we reconcile: deleted files drop their
    /// bare line, and files that reappear get their bare line re-added.
    #[test]
    fn test_reconcile_bare_lines_with_existence() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let track1 = music_dir.join("track1.mp3");
        let track2 = music_dir.join("track2.mp3");
        std::fs::write(&track1, b"test")?;
        std::fs::write(&track2, b"test")?;

        let playlist_path = temp_dir.path().join("playlist.m3u");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.add_positive(&track1, &test_features())?;
        playlist.add_positive(&track2, &test_features())?;

        // Both files exist: both bare lines should be present.
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("\nmusic/track1.mp3\n"));
        assert!(content.contains("\nmusic/track2.mp3\n"));

        // Delete track1. On reopen, its bare line is *not* dropped
        // automatically (reconcile is opt-in now); it is only removed once
        // the user runs `reconcile`. The `#{...}` metadata line is preserved
        // either way, so the classification survives for training.
        std::fs::remove_file(&track1)?;
        let playlist = M3uPlaylist::open(&playlist_path)?;
        let content = std::fs::read_to_string(&playlist_path)?;
        // Before reconciling, the stale bare line for track1 is still on disk.
        assert!(content.contains("\nmusic/track1.mp3\n"));
        assert!(content.contains("\nmusic/track2.mp3\n"));

        // Explicit reconcile drops the bare line for the deleted file.
        playlist.reconcile()?;
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(!content.contains("\nmusic/track1.mp3\n"));
        assert!(content.contains("\nmusic/track2.mp3\n"));
        // Both entries are still loaded in memory.
        assert_eq!(playlist.entries().len(), 2);
        assert!(playlist.entries()[0].is_positive());
        assert!(playlist.entries()[1].is_positive());

        // Re-create track1. On reopen its bare line is still absent until we
        // reconcile, which re-adds it so VLC picks it up again.
        std::fs::write(&track1, b"test")?;
        let playlist = M3uPlaylist::open(&playlist_path)?;
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(!content.contains("\nmusic/track1.mp3\n"));
        playlist.reconcile()?;
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(content.contains("\nmusic/track1.mp3\n"));
        assert!(content.contains("\nmusic/track2.mp3\n"));

        Ok(())
    }

    /// Opening a playlist never rewrites it: reconcile is an explicit,
    /// opt-in operation. Calling `reconcile` on an already-consistent playlist
    /// is a no-op on disk.
    #[test]
    fn test_reconcile_is_noop_when_consistent() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let track1 = music_dir.join("track1.mp3");
        std::fs::write(&track1, b"test")?;

        let playlist_path = temp_dir.path().join("playlist.m3u");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.add_positive(&track1, &test_features())?;

        let before = std::fs::read_to_string(&playlist_path)?;
        let playlist = M3uPlaylist::open(&playlist_path)?;
        playlist.reconcile()?;
        let after = std::fs::read_to_string(&playlist_path)?;
        assert_eq!(before, after);

        Ok(())
    }

    /// Media features passed to `add_positive`/`add_negative` are persisted to
    /// disk and survive a reload: they round-trip through the `#{...}` metadata
    /// line back onto the loaded `EntryMeta`. This is the core of "plumb
    /// MediaFeatures through and persist with classification results".
    #[test]
    fn test_features_persist_with_classification() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let track1 = music_dir.join("track1.mp3");
        let track2 = music_dir.join("track2.mp3");
        std::fs::write(&track1, b"test")?;
        std::fs::write(&track2, b"test")?;

        let playlist_path = temp_dir.path().join("playlist.m3u");
        let mut playlist = M3uPlaylist::open(&playlist_path)?;
        let features = test_features();
        playlist.add_positive(&track1, &features)?;
        playlist.add_negative(&track2, &features)?;

        // The serialized short key is present on disk for both entries.
        let content = std::fs::read_to_string(&playlist_path)?;
        assert!(
            content.contains("\"m\""),
            "features should be serialized under the `m` key; got:\n{}",
            content
        );

        // Reopen and confirm the features round-trip onto the entries.
        let playlist = M3uPlaylist::open(&playlist_path)?;
        assert_eq!(playlist.entries().len(), 2);
        for entry in playlist.entries() {
            assert_eq!(
                entry.features, features,
                "features must round-trip through the playlist"
            );
        }

        Ok(())
    }

    /// Older playlists written before the `features` field existed still load:
    /// the missing field defaults to an all-zero `MediaFeatures` rather than
    /// failing deserialization.
    #[test]
    fn test_legacy_entries_default_features() -> Result<(), Error> {
        let temp_dir = tempdir()?;
        let music_dir = temp_dir.path().join("music");
        std::fs::create_dir(&music_dir)?;

        let track1 = music_dir.join("track1.mp3");
        std::fs::write(&track1, b"test")?;

        let playlist_path = temp_dir.path().join("playlist.m3u");
        // A legacy entry with no `m` field, plus its bare filename line.
        let legacy =
            "#EXTM3U\n#{\"f\":\"music/track1.mp3\",\"a\":1700000000,\"s\":1}\nmusic/track1.mp3\n"
                .to_string();
        std::fs::write(&playlist_path, legacy)?;

        let playlist = M3uPlaylist::open(&playlist_path)?;
        assert_eq!(playlist.entries().len(), 1);
        assert!(playlist.entries()[0].is_positive());
        assert_eq!(
            playlist.entries()[0].features,
            MediaFeatures::default(),
            "missing features field must default rather than fail to load"
        );

        Ok(())
    }
}
