//! Shared, append-only on-disk cache for extracted media features produced by
//! `ffprobe`.
//!
//! See `docs/ffprobe-cache.md` for the full design. In short: one JSONL file
//! sits next to the playlist ([`CACHE_FILE_NAME`]). Each line is keyed by
//! (path relative to the playlist root, mtime in whole seconds, size). A run
//! loads the file, probes only the files without an entry (via the
//! [`Probe`](crate::ffprobe::Probe) trait), and appends the new entries. The
//! file is never rewritten, compacted, or expired. A torn line, a failed
//! probe, or a cache I/O error is logged and skipped and never aborts a run.

use crate::ffprobe::Probe;
use crate::logging::time_it;
use crate::path::{AbsPath, PathDisplayContext, normalize_path};
use crate::walk::File as WalkFile;
use log::*;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Name of the cache file, stored in the playlist root (the M3U's parent dir).
pub const CACHE_FILE_NAME: &str = ".classi-cine-ffprobe-cache.jsonl";

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

/// Extracted ffprobe features, stored with short serde keys for compactness.
/// Only raw, non-derivable values are persisted; aspect ratio and bitrate are
/// derived by the classifier at read time. See `docs/ffprobe-cache.md`.
///
/// Implements [`Default`] (all-zero / empty values) so it can be a required
/// field on structs that are deserialized from older data which predates the
/// field, via `#[serde(default)]`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaFeatures {
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
    /// File size in bytes, sourced from `walk::File::size` (not ffprobe).
    /// Doubles as the `FileSizeClassifier` input once `MediaFeatures` is
    /// embedded in `playlist::EntryMeta`, and is the numerator for the derived
    /// bitrate.
    #[serde(rename = "s")]
    pub file_size: u64,
    /// Raw ffprobe video codec name, e.g. "h264", "hevc", "vp9".
    #[serde(rename = "vc", default)]
    pub video_codec: String,
    /// Raw ffprobe audio codec name, e.g. "aac", "ac3".
    #[serde(rename = "ac", default)]
    pub audio_codec: String,
    /// Duration in seconds (float). Near-unique per title, so any classifier
    /// MUST discretize (bucket) this before using it as a feature. Required: a
    /// probe that yields no duration is a probe failure.
    #[serde(rename = "d")]
    pub duration_secs: f64,
    /// Average frame rate (fps). Discretize before use. Optional because
    /// ffprobe occasionally omits it.
    #[serde(rename = "fps", default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}

/// One record per file, serialized as a single JSONL line. Mirrors the serde
/// conventions of `playlist::EntryMeta` (short keys, aliases for readability)
/// but is a separate type. The file size lives in `features.file_size`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// Path relative to the playlist root (`../...` if outside it).
    #[serde(rename = "p", alias = "path")]
    pub path: String,
    /// File mtime in whole unix seconds.
    #[serde(rename = "m", alias = "mtime")]
    pub mtime: i64,
    /// Extracted ffprobe features.
    #[serde(rename = "f", alias = "features")]
    pub features: MediaFeatures,
}

impl CacheEntry {
    pub fn key(&self) -> CacheKey {
        CacheKey {
            path: self.path.clone(),
            mtime: self.mtime,
            size: self.features.file_size,
        }
    }
}

/// Identity of a cached file: (path relative to the playlist root, mtime, size).
///
/// The mtime is deliberately truncated to whole unix seconds: the cache file
/// is shared between machines over sshfs, where SFTP v3 transfers mtime as a
/// 32-bit seconds value, while the local machine sees nanoseconds. Comparing
/// anything finer would never match across machines.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    path: String,
    mtime: i64,
    size: u64,
}

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

/// The ffprobe feature cache for one playlist root. [`Cache::populate`] never
/// returns an error: any failure degrades to probing without caching.
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// A cache for the playlist root `root` (the M3U's parent directory).
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Path of the cache file.
    pub fn file_path(&self) -> PathBuf {
        self.root.join(CACHE_FILE_NAME)
    }

    /// Build the lookup key for a file, relativizing the path against the
    /// playlist root exactly as M3U entries are (so the cache and the playlist
    /// agree).
    pub fn key_for(&self, abs_path: &Path, mtime: SystemTime, size: u64) -> CacheKey {
        self.key_for_secs(abs_path, unix_secs(mtime), size)
    }

    fn key_for_secs(&self, abs_path: &Path, mtime: i64, size: u64) -> CacheKey {
        CacheKey {
            path: AbsPath::from_abs_path(abs_path)
                .to_string(&PathDisplayContext::RelativeTo(self.root.clone())),
            mtime,
            size,
        }
    }

    /// load -> compute missing -> probe in parallel -> append.
    ///
    /// Returns a map of every known entry (loaded + freshly probed). A file
    /// whose probe failed is simply absent. The cache file is only created if
    /// there is something to append.
    pub fn populate<P: Probe + Sync>(
        &self,
        files: &[WalkFile],
        probe: &P,
    ) -> HashMap<CacheKey, MediaFeatures> {
        info!(
            "cache: file={} collected={}",
            self.file_path().display(),
            files.len()
        );

        let mut map = time_it!("cache: load", { self.load() });
        info!("cache: loaded {} entries", map.len());

        let missing: Vec<&WalkFile> = files
            .iter()
            .filter(|f| !map.contains_key(&self.key_for(&f.path, f.created, f.size)))
            .collect();
        info!("cache: {} missing files to probe", missing.len());
        if missing.is_empty() {
            return map;
        }

        let probed = time_it!("cache: probe + append missing", {
            let successes = AtomicUsize::new(0);
            let failures = AtomicUsize::new(0);
            let (tx, rx) = mpsc::channel::<CacheEntry>();
            let mut writer = Appender::new(&self.file_path());
            // Single writer thread owns the file; probes feed it via channel.
            let writer_handle = std::thread::spawn(move || -> Vec<CacheEntry> {
                let mut probed = Vec::new();
                for entry in rx {
                    writer.append(&entry);
                    probed.push(entry);
                }
                writer.sync();
                probed
            });

            missing.par_iter().for_each(|f| match probe.probe(f) {
                Ok(mut features) => {
                    features.file_size = f.size;
                    let key = self.key_for(&f.path, f.created, f.size);
                    let entry = CacheEntry {
                        path: key.path,
                        mtime: key.mtime,
                        features,
                    };
                    let _ = tx.send(entry);
                    let done = successes.fetch_add(1, Ordering::Relaxed) + 1;
                    if done.is_multiple_of(100) {
                        info!(
                            "cache: probing... {}/{} ({}%)",
                            done,
                            missing.len(),
                            done * 100 / missing.len().max(1)
                        );
                    }
                }
                Err(e) => {
                    warn!("cache: probe failed for {}: {}", f.path.display(), e);
                    failures.fetch_add(1, Ordering::Relaxed);
                }
            });
            drop(tx);
            let probed = writer_handle.join().unwrap_or_default();
            info!(
                "cache: probed {} missing files (success={} failed={})",
                missing.len(),
                successes.load(Ordering::Relaxed),
                failures.load(Ordering::Relaxed)
            );
            probed
        });

        for e in probed {
            map.insert(e.key(), e.features);
        }
        map
    }

    /// Read the whole cache file. Lines are parsed in parallel; unparseable
    /// (e.g. torn) lines are skipped; for duplicate keys the last line wins.
    /// A missing or unreadable file yields an empty map.
    fn load(&self) -> HashMap<CacheKey, MediaFeatures> {
        load_entries(&self.file_path())
            .into_iter()
            .map(|e| (e.key(), e.features))
            .collect()
    }

    /// Used by `move`: copy this cache's entries into the cache at
    /// `new_root`, rebasing each path (old_root.join(p), then relative to
    /// new_root). Entries already present in the destination are skipped. The
    /// source file is left untouched. Returns the number of entries appended.
    pub fn rebase_into(&self, new_root: &Path) -> std::io::Result<usize> {
        let dest = Cache::new(new_root);
        let existing = dest.load();
        let mut writer = Appender::new(&dest.file_path());
        let mut count = 0;
        for e in load_entries(&self.file_path()) {
            let abs = normalize_path(&self.root.join(&e.path));
            let key = dest.key_for_secs(&abs, e.mtime, e.features.file_size);
            if existing.contains_key(&key) {
                continue;
            }
            writer.append(&CacheEntry {
                path: key.path,
                mtime: key.mtime,
                features: e.features,
            });
            count += 1;
        }
        writer.sync();
        Ok(count)
    }
}

/// Parse every valid line of a cache file, deduplicated by key with the last
/// occurrence winning. A missing or unreadable file yields no entries.
fn load_entries(path: &Path) -> Vec<CacheEntry> {
    let content = match fs::read(path) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!("cache: cannot read {}: {}", path.display(), e);
            }
            return Vec::new();
        }
    };
    let lines: Vec<&[u8]> = content
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    let parsed: Vec<CacheEntry> = lines
        .par_iter()
        .filter_map(|l| serde_json::from_slice::<CacheEntry>(l).ok())
        .collect();
    let mut last: HashMap<CacheKey, usize> = HashMap::with_capacity(parsed.len());
    for (i, e) in parsed.iter().enumerate() {
        last.insert(e.key(), i);
    }
    parsed
        .into_iter()
        .enumerate()
        .filter(|(i, e)| last.get(&e.key()) == Some(i))
        .map(|(_, e)| e)
        .collect()
}

// ---------------------------------------------------------------------------
// Appending
// ---------------------------------------------------------------------------

/// Appends complete lines to the cache file (`create(true).append(true)`), one
/// `write_all` per entry so concurrent machines under `O_APPEND` do not
/// interleave within a line. The file is opened lazily on the first append, so
/// nothing is created when there is nothing to write. An open or write error
/// is logged once and disables further writes (probing continues uncached).
struct Appender {
    path: PathBuf,
    file: Option<fs::File>,
    failed: bool,
    /// True if the file did not end in a newline (a torn line from a crashed
    /// writer): the next write starts with `\n` so the torn fragment cannot
    /// swallow a good entry.
    needs_newline: bool,
}

impl Appender {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            file: None,
            failed: false,
            needs_newline: false,
        }
    }

    fn open(&mut self) -> Option<&mut fs::File> {
        if self.file.is_none() && !self.failed {
            self.needs_newline = ends_without_newline(&self.path);
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                Ok(f) => self.file = Some(f),
                Err(e) => {
                    error!(
                        "cache: cannot open {} for append: {}; continuing without caching",
                        self.path.display(),
                        e
                    );
                    self.failed = true;
                }
            }
        }
        self.file.as_mut()
    }

    /// Append one entry and flush so Ctrl-C keeps progress.
    fn append(&mut self, entry: &CacheEntry) {
        let mut line = match serde_json::to_string(entry) {
            Ok(s) => s,
            Err(e) => {
                warn!("cache: cannot serialize entry: {}", e);
                return;
            }
        };
        line.push('\n');
        if self.open().is_none() {
            return;
        }
        if self.needs_newline {
            line.insert(0, '\n');
            self.needs_newline = false;
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if let Err(e) = file.write_all(line.as_bytes()).and_then(|_| file.flush()) {
            error!("cache: write failed: {}", e);
            self.file = None;
            self.failed = true;
        }
    }

    fn sync(&self) {
        if let Some(f) = &self.file {
            let _ = f.sync_all();
        }
    }
}

/// True if the file exists, is non-empty, and its last byte is not `\n`.
/// Seeks to the last byte rather than reading the whole cache.
fn ends_without_newline(path: &Path) -> bool {
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut last = [0u8; 1];
    f.seek(SeekFrom::End(-1)).is_ok() && f.read_exact(&mut last).is_ok() && last[0] != b'\n'
}

/// Convert a `SystemTime` to unix seconds (whole-second granularity).
fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::ffprobe::Probe;
    use std::time::Duration;
    use tempfile::tempdir;

    /// A deterministic stub `Probe` that derives features from the file's size
    /// so tests can exercise the populate flow without the ffprobe binary.
    struct StubProbe;

    impl Probe for StubProbe {
        fn probe(&self, file: &WalkFile) -> Result<MediaFeatures, Error> {
            Ok(MediaFeatures {
                width: 1920,
                height: 1080,
                file_size: file.size,
                video_codec: "h264".into(),
                audio_codec: "aac".into(),
                duration_secs: file.size as f64 / 1_000_000.0,
                fps: Some(24.0),
            })
        }
    }

    /// A stub `Probe` that always fails.
    struct FailingProbe;

    impl Probe for FailingProbe {
        fn probe(&self, file: &WalkFile) -> Result<MediaFeatures, Error> {
            Err(Error::ProbeFailed {
                path: file.path.display().to_string(),
                reason: "stub failure".into(),
            })
        }
    }

    fn mtime(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn walk_file(dir: &Path, name: &str, size: u64, mtime: SystemTime) -> WalkFile {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![0u8; size as usize]).unwrap();
        WalkFile {
            path: AbsPath::from_abs_path(&path),
            size,
            created: mtime,
        }
    }

    fn lines(root: &Path) -> Vec<String> {
        std::fs::read_to_string(root.join(CACHE_FILE_NAME))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    fn get<'a>(
        c: &Cache,
        m: &'a HashMap<CacheKey, MediaFeatures>,
        f: &WalkFile,
    ) -> Option<&'a MediaFeatures> {
        m.get(&c.key_for(&f.path, f.created, f.size))
    }

    #[test]
    fn probes_and_persists() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let files = vec![
            walk_file(dir.path(), "a.mp4", 1000, mtime(1_700_000_000)),
            walk_file(dir.path(), "sub/b.mp4", 2000, mtime(1_700_000_001)),
        ];
        assert!(!dir.path().join(CACHE_FILE_NAME).exists());
        let map = cache.populate(&files, &StubProbe);
        assert_eq!(map.len(), 2);
        assert_eq!(get(&cache, &map, &files[1]).unwrap().file_size, 2000);
        assert_eq!(lines(dir.path()).len(), 2);
        // A fresh cache object sees the persisted entries.
        let reloaded = Cache::new(dir.path()).populate(&files, &FailingProbe);
        assert_eq!(reloaded, map);
    }

    #[test]
    fn nothing_to_append_creates_no_file() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let files = vec![walk_file(dir.path(), "a.mp4", 10, mtime(5))];
        let map = cache.populate(&files, &FailingProbe);
        assert!(map.is_empty());
        assert!(!dir.path().join(CACHE_FILE_NAME).exists());
    }

    #[test]
    fn cached_entries_are_not_reprobed() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let files = vec![walk_file(dir.path(), "a.mp4", 1000, mtime(100))];
        cache.populate(&files, &StubProbe);
        let map = cache.populate(&files, &FailingProbe);
        assert!(get(&cache, &map, &files[0]).is_some());
        assert_eq!(lines(dir.path()).len(), 1, "no new line appended");
    }

    #[test]
    fn unmatched_entries_survive_subset_scan() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let a = walk_file(dir.path(), "a.mp4", 1000, mtime(100));
        let b = walk_file(dir.path(), "b.mp4", 2000, mtime(200));
        cache.populate(&[a, b], &StubProbe);
        let before = std::fs::read(dir.path().join(CACHE_FILE_NAME)).unwrap();
        let c = walk_file(dir.path(), "c.mp4", 3000, mtime(300));
        let map = cache.populate(std::slice::from_ref(&c), &StubProbe);
        assert_eq!(map.len(), 3, "entries for a and b were kept");
        let after = std::fs::read(dir.path().join(CACHE_FILE_NAME)).unwrap();
        assert!(after.starts_with(&before), "file only grows by appending");
        assert_eq!(lines(dir.path()).len(), 3);
    }

    #[test]
    fn torn_and_corrupt_lines_are_skipped() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let a = walk_file(dir.path(), "a.mp4", 1000, mtime(100));
        cache.populate(std::slice::from_ref(&a), &StubProbe);
        let path = dir.path().join(CACHE_FILE_NAME);
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("not json\n{\"p\":\"zzz\",\"m\":1,\"f\":{\"w\":\n");
        content.push_str("{\"p\":\"torn.mp4\",\"m\":5"); // no trailing newline
        std::fs::write(&path, content).unwrap();

        let b = walk_file(dir.path(), "b.mp4", 2000, mtime(200));
        let map = cache.populate(&[a.clone(), b.clone()], &StubProbe);
        assert!(get(&cache, &map, &a).is_some());
        assert!(get(&cache, &map, &b).is_some());
        // The new entry must not have been glued onto the torn fragment.
        let again = Cache::new(dir.path()).populate(&[a, b], &FailingProbe);
        assert_eq!(again.len(), 2);
    }

    #[test]
    fn duplicate_keys_last_wins() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let a = walk_file(dir.path(), "a.mp4", 1000, mtime(100));
        let mk = |codec: &str| {
            let mut f = StubProbe.probe(&a).unwrap();
            f.video_codec = codec.into();
            serde_json::to_string(&CacheEntry {
                path: "a.mp4".into(),
                mtime: 100,
                features: f,
            })
            .unwrap()
        };
        std::fs::write(
            dir.path().join(CACHE_FILE_NAME),
            format!("{}\n{}\n", mk("old"), mk("new")),
        )
        .unwrap();
        let map = cache.populate(std::slice::from_ref(&a), &FailingProbe);
        assert_eq!(get(&cache, &map, &a).unwrap().video_codec, "new");
    }

    #[test]
    fn changed_mtime_or_size_reprobes() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let a = walk_file(dir.path(), "a.mp4", 1000, mtime(100));
        cache.populate(std::slice::from_ref(&a), &StubProbe);
        let newer = walk_file(dir.path(), "a.mp4", 1000, mtime(101));
        let map = cache.populate(std::slice::from_ref(&newer), &FailingProbe);
        assert!(get(&cache, &map, &newer).is_none());
        let bigger = walk_file(dir.path(), "a.mp4", 1001, mtime(100));
        let map = cache.populate(std::slice::from_ref(&bigger), &StubProbe);
        assert_eq!(get(&cache, &map, &bigger).unwrap().file_size, 1001);
        assert_eq!(lines(dir.path()).len(), 2);
    }

    #[test]
    fn subsecond_mtime_is_ignored() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let p = dir.path().join("a.mp4");
        assert_eq!(
            cache.key_for(&p, mtime(100) + Duration::from_nanos(123_456_789), 5),
            cache.key_for(&p, mtime(100), 5)
        );
    }

    #[test]
    fn serde_keys() {
        let e = CacheEntry {
            path: "a/b.mp4".into(),
            mtime: 42,
            features: MediaFeatures {
                width: 1,
                height: 2,
                file_size: 3,
                video_codec: "h264".into(),
                audio_codec: "aac".into(),
                duration_secs: 4.0,
                fps: None,
            },
        };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["f", "m", "p"]);
        assert_eq!(v["p"], "a/b.mp4");
        assert_eq!(v["m"], 42);
        assert_eq!(v["f"]["s"], 3);
        let back: CacheEntry = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn key_for_is_relative_with_parent_dirs_outside_root() {
        let cache = Cache::new(Path::new("/lib/playlists"));
        let m = mtime(7);
        assert_eq!(
            cache
                .key_for(Path::new("/lib/playlists/x/a.mp4"), m, 1)
                .path,
            "x/a.mp4"
        );
        assert_eq!(
            cache.key_for(Path::new("/lib/movies/b.mp4"), m, 1).path,
            "../movies/b.mp4"
        );
    }

    #[test]
    fn move_rebases_entries_and_keeps_old_cache() {
        let tmp = tempdir().unwrap();
        let old_root = tmp.path().join("old");
        let new_root = tmp.path().join("new");
        std::fs::create_dir_all(&new_root).unwrap();
        let inside = walk_file(&old_root, "a.mp4", 1000, mtime(100));
        let outside = walk_file(tmp.path(), "movies/b.mp4", 2000, mtime(200));
        let old = Cache::new(&old_root);
        old.populate(&[inside.clone(), outside.clone()], &StubProbe);
        let old_bytes = std::fs::read(old.file_path()).unwrap();

        assert_eq!(old.rebase_into(&new_root).unwrap(), 2);
        assert_eq!(std::fs::read(old.file_path()).unwrap(), old_bytes);

        let new = Cache::new(&new_root);
        let map = new.populate(&[inside.clone(), outside.clone()], &FailingProbe);
        assert!(get(&new, &map, &inside).is_some());
        assert!(get(&new, &map, &outside).is_some());
        assert!(
            new.key_for(&inside.path, inside.created, inside.size)
                .path
                .starts_with("../old/")
        );
        // Repeating the move adds nothing.
        assert_eq!(old.rebase_into(&new_root).unwrap(), 0);
    }
}
