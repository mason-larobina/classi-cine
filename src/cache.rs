//! Persistent, sharded on-disk cache for extracted media features produced by
//! `ffprobe`.
//!
//! See `docs/ffprobe-cache.md` for the full design. In short: each (path,
//! mtime, size) triple is hashed into a stable `entry_hash` that is the
//! entry's sole identity. Surviving entries are rewritten as a fresh
//! generation of `<seq>.jsonl` shards every startup; missing files are
//! re-probed (via the [`Probe`](crate::ffprobe::Probe) trait) and appended to
//! the new generation. The whole flow is resilient: a corrupt shard, a failed
//! probe, or a cache I/O error is logged and skipped and never aborts a run.

use crate::ffprobe::Probe;
use crate::logging::time_it;
use crate::walk::File as WalkFile;
use log::*;
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File as FsFile, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Target shard size. A shard is rolled to a new file when appending the next
/// line would exceed this byte count. ~170 bytes/line → ~49 000 entries/shard.
pub const TARGET_SHARD_BYTES: usize = 8 * 1024 * 1024;

/// Seconds per day, used to convert `--cache-ttl-days` to a TTL in seconds.
const SECS_PER_DAY: i64 = 86_400;

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

/// One record per file, serialized as a single JSONL line in a shard. Mirrors
/// the serde conventions of `playlist::EntryMeta` (short keys, aliases for
/// readability, tolerant of unknown fields) but is a separate type.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// The per-file lookup key (SHA256-hex of path+mtime+size). The entry's
    /// sole identity: the plaintext path/mtime/size are NOT stored.
    #[serde(rename = "k", alias = "key")]
    pub key: String,
    /// Unix seconds of the most recent startup at which this entry's key was
    /// present among the collected files. Drives TTL expiry.
    #[serde(rename = "u", alias = "last_used")]
    pub last_used: i64,
    /// Extracted ffprobe features.
    #[serde(rename = "f", alias = "features")]
    pub features: MediaFeatures,
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Compute the per-file lookup key:
///
/// ```text
/// entry_hash = SHA256( canonical_abs_path || ":" || mtime_secs || ":" || size )
/// ```
///
/// `mtime` is the file's modification time (`walk::File::created`); `size` is
/// the file size in bytes. The plaintext values are NOT stored on the entry —
/// they are fully bound into the key, so a changed or moved file yields a
/// different key and the old entry simply expires by TTL.
pub fn entry_hash(path: &Path, mtime: SystemTime, size: u64) -> String {
    let mtime_secs = mtime
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update(b":");
    hasher.update(mtime_secs.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(size.to_string().as_bytes());
    hex_encode(&hasher.finalize())
}

/// Lowercase hex encoding of a byte slice (avoids pulling in the `hex` crate).
fn hex_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(TABLE[(b >> 4) as usize] as char);
        out.push(TABLE[(b & 0x0f) as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Cache directory
// ---------------------------------------------------------------------------

/// Resolve the XDG cache directory for this tool:
/// `$XDG_CACHE_HOME/classi-cine/ffprobe`, falling back to
/// `$HOME/.cache/classi-cine/ffprobe` when `XDG_CACHE_HOME` is unset/empty,
/// via the `dirs` crate.
pub fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("classi-cine").join("ffprobe"))
}

// ---------------------------------------------------------------------------
// Cache: load / compact / write / probe
// ---------------------------------------------------------------------------

/// The ffprobe feature cache. Owns the on-disk shard directory and the TTL
/// window. [`Cache::populate`] runs the full five-phase startup pass and never
/// returns an error — it degrades to an empty cache (and a full re-probe) on
/// any failure, which is the correct recovery.
pub struct Cache {
    dir: PathBuf,
    /// TTL in seconds. `i64::MAX` disables expiry (the `--cache-ttl-days 0`
    /// case).
    ttl_secs: i64,
}

impl Cache {
    /// Construct a cache rooted at `dir` with the given TTL in days. A
    /// `ttl_days` of `0` disables expiry entirely.
    pub fn new(dir: PathBuf, ttl_days: u32) -> Self {
        let ttl_secs = if ttl_days == 0 {
            i64::MAX
        } else {
            ttl_days as i64 * SECS_PER_DAY
        };
        Self { dir, ttl_secs }
    }

    /// Construct a cache at the default [`cache_dir`] location.
    pub fn with_default_dir(ttl_days: u32) -> Option<Self> {
        Some(Self::new(cache_dir()?, ttl_days))
    }

    /// The full startup populate pass: **load → compact → write-survivors →
    /// delete → probe+write-missing**. See `docs/ffprobe-cache.md`.
    ///
    /// Never returns an error: a totally unreadable cache dir degrades to an
    /// empty cache and a full re-probe. Per-file probe failures are logged and
    /// the file is left without an entry (retried next run).
    ///
    /// Returns a lookup map of `entry_hash → MediaFeatures` covering every
    /// entry now persisted on disk (survivors + freshly probed). Callers use
    /// it to attach features to their own per-file records. A file whose probe
    /// failed is simply absent from the map.
    pub fn populate<P: Probe + Sync>(
        &self,
        files: &[WalkFile],
        probe: &P,
    ) -> HashMap<String, MediaFeatures> {
        let now = unix_secs(SystemTime::now());
        info!(
            "cache: dir={} ttl_days={} collected={}",
            self.dir.display(),
            if self.ttl_secs == i64::MAX {
                0
            } else {
                self.ttl_secs / SECS_PER_DAY
            },
            files.len()
        );

        // (1) LOAD
        let entries = time_it!("cache: load", { self.load_all() });
        info!("cache: loaded {} entries from disk", entries.len());

        // (2) COMPACT (pure in-memory)
        let (survivors, missing) = time_it!("cache: compact", {
            let live_keys: HashSet<String> = files
                .iter()
                .map(|f| entry_hash(&f.path, f.created, f.size))
                .collect();
            let mut survivors: Vec<CacheEntry> = Vec::with_capacity(entries.len());
            let mut survivor_keys: HashSet<String> = HashSet::new();
            let mut refreshed = 0usize;
            let mut kept_fresh = 0usize;
            let mut expired = 0usize;
            for mut entry in entries {
                if live_keys.contains(&entry.key) {
                    entry.last_used = now; // refresh: file present & unchanged
                    survivor_keys.insert(entry.key.clone());
                    survivors.push(entry);
                    refreshed += 1;
                } else if now.saturating_sub(entry.last_used) < self.ttl_secs {
                    survivors.push(entry); // unmatched but fresh: keep as-is
                    kept_fresh += 1;
                } else {
                    expired += 1; // unmatched and past TTL -> dropped
                }
            }
            info!(
                "cache: compacted survivors={} (refreshed={} kept_fresh={} expired={})",
                survivors.len(),
                refreshed,
                kept_fresh,
                expired
            );
            let missing: Vec<&WalkFile> = files
                .iter()
                .filter(|f| !survivor_keys.contains(&entry_hash(&f.path, f.created, f.size)))
                .collect();
            info!("cache: {} missing files to probe", missing.len());
            (survivors, missing)
        });

        // Ensure the directory exists before writing. A failure here means we
        // cannot cache at all: log and bail (probes are skipped — they would
        // have nowhere to write).
        if let Err(e) = fs::create_dir_all(&self.dir) {
            error!("cache: cannot create dir {}: {}", self.dir.display(), e);
            return HashMap::new();
        }

        let base = self.max_seq().unwrap_or(0);

        // (3) WRITE survivors to a fresh generation (base+1, base+2, …).
        let writer = time_it!("cache: write survivors", {
            let mut writer = match ShardWriter::new(&self.dir, base + 1) {
                Ok(w) => w,
                Err(e) => {
                    error!("cache: cannot open first shard: {}", e);
                    return HashMap::new();
                }
            };
            for e in &survivors {
                writer.emit(e);
            }
            writer.fsync();
            info!(
                "cache: wrote {} survivors to generation >= {}",
                survivors.len(),
                base + 1
            );
            writer
        });

        // (4) DELETE the old generation (seq <= base) now that the new
        //     generation is persisted. This is the delete-before-probe
        //     boundary.
        time_it!("cache: delete old generation", {
            let deleted = self.delete_generation(base);
            fsync_dir(&self.dir);
            info!("cache: deleted {} old shards (seq <= {})", deleted, base);
        });

        // (5) PROBE + WRITE missing (streamed). Probe results are fed back to
        //     the single owning writer via a channel so shard rolling and byte
        //     tracking stay on one thread; the par_iter over `missing` supplies
        //     the work. The writer handle (with its live `seq`/`cur_bytes`) is
        //     moved into the writer thread so appends continue in the same top
        //     shard left open from step (3) — no fresh handle, no lost byte
        //     count, so exact-byte rolling still holds.
        let probed = time_it!("cache: probe + write missing", {
            let successes = AtomicUsize::new(0);
            let failures = AtomicUsize::new(0);
            let (tx, rx) = mpsc::channel::<CacheEntry>();
            let writer_handle = std::thread::spawn(move || -> std::io::Result<Vec<CacheEntry>> {
                let mut writer = writer;
                // Collect the freshly probed entries alongside writing them so the
                // returned features map can be assembled without a second disk
                // read. Survivors are already in `survivors`; the probed entries
                // are only observable here, inside the writer thread.
                let mut probed: Vec<CacheEntry> = Vec::new();
                for entry in rx {
                    writer.emit(&entry);
                    probed.push(entry);
                }
                writer.fsync();
                Ok(probed)
            });

            missing.par_iter().for_each(|f| {
                match probe.probe(f) {
                    Ok(features) => {
                        let entry = CacheEntry {
                            key: entry_hash(&f.path, f.created, f.size),
                            last_used: now,
                            features,
                        };
                        if tx.send(entry).is_err() {
                            // Writer thread died; stop probing.
                        }
                        let done = successes.fetch_add(1, Ordering::Relaxed) + 1;
                        if done % 100 == 0 {
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
                }
            });
            drop(tx);
            let probed = writer_handle
                .join()
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or_default();
            fsync_dir(&self.dir);
            info!(
                "cache: probed {} missing files (success={} failed={})",
                missing.len(),
                successes.load(Ordering::Relaxed),
                failures.load(Ordering::Relaxed)
            );
            probed
        });

        // (6) BUILD the returned features map: survivors (refreshed + kept
        //     fresh) plus the freshly probed entries. Survivors are inserted
        //     first so a freshly probed duplicate (the same key appearing in
        //     both, e.g. across runs) wins. Extras for files no longer present
        //     are harmless — callers look up only their current files.
        let mut map: HashMap<String, MediaFeatures> =
            HashMap::with_capacity(survivors.len() + probed.len());
        for e in probed {
            map.insert(e.key, e.features);
        }
        for e in &survivors {
            map.entry(e.key.clone())
                .or_insert_with(|| e.features.clone());
        }
        map
    }

    // -- internal helpers ---------------------------------------------------

    /// (1) Load every `*.jsonl` shard in parallel (one rayon task per file;
    /// within each shard, split the JSONL lines and deserialize in parallel).
    /// Resilient: a shard that fails to read or parse is discarded. Dedup by
    /// key, keeping the entry with the greatest `last_used` (recovers a crash
    /// mid-rewrite that left an old generation and a partial new one).
    fn load_all(&self) -> Vec<CacheEntry> {
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(_) => return Vec::new(),
        };
        let shard_paths: Vec<PathBuf> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
            .collect();
        let per_shard: Vec<Vec<CacheEntry>> =
            shard_paths.par_iter().map(|p| load_shard(p)).collect();

        let mut by_key: HashMap<String, CacheEntry> = HashMap::new();
        for entry in per_shard.into_iter().flatten() {
            match by_key.get(&entry.key) {
                Some(existing) if existing.last_used >= entry.last_used => {}
                _ => {
                    by_key.insert(entry.key.clone(), entry);
                }
            }
        }
        by_key.into_values().collect()
    }

    /// The highest shard sequence number currently on disk, or `None` if the
    /// directory holds no parseable `<seq>.jsonl` file.
    fn max_seq(&self) -> Option<u64> {
        let rd = fs::read_dir(&self.dir).ok()?;
        rd.filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let stem = p.file_stem()?.to_str()?;
                stem.parse::<u64>().ok()
            })
            .max()
    }

    /// Delete every shard whose sequence number is `<= upper_inclusive`.
    /// Individual delete failures are logged and skipped. Returns the count of
    /// shards successfully removed (for status logging).
    fn delete_generation(&self, upper_inclusive: u64) -> usize {
        let rd = match fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) => {
                warn!("cache: cannot list dir for delete: {}", e);
                return 0;
            }
        };
        let mut deleted = 0usize;
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(seq) = stem.parse::<u64>() else {
                continue;
            };
            if seq <= upper_inclusive {
                if let Err(e) = fs::remove_file(&path) {
                    warn!("cache: cannot remove old shard {}: {}", path.display(), e);
                } else {
                    deleted += 1;
                }
            }
        }
        deleted
    }
}

// ---------------------------------------------------------------------------
// Shard loading
// ---------------------------------------------------------------------------

/// Load and deserialize every `CacheEntry` from a single JSONL shard. Any I/O
/// error, malformed line, or schema mismatch discards just that line (or the
/// whole file on a read failure) — never propagates an error.
fn load_shard(path: &Path) -> Vec<CacheEntry> {
    let content = match fs::read(path) {
        Ok(c) => c,
        Err(e) => {
            warn!("cache: cannot read shard {}: {}", path.display(), e);
            return Vec::new();
        }
    };
    // Split into self-contained lines and deserialize in parallel (JSONL's
    // self-delimiting property gives within-shard parallelism on top of the
    // across-shard parallelism).
    let lines: Vec<&[u8]> = content
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .par_iter()
        .filter_map(|l| serde_json::from_slice::<CacheEntry>(l).ok())
        .collect()
}

// ---------------------------------------------------------------------------
// Shard writing
// ---------------------------------------------------------------------------

/// Appends serialized `CacheEntry` lines to `<seq>.jsonl` shards, rolling to a
/// new file when the next line would exceed [`TARGET_SHARD_BYTES`]. Tracks the
/// exact byte count so rolling needs no per-entry estimate.
struct ShardWriter {
    dir: PathBuf,
    seq: u64,
    file: FsFile,
    cur_bytes: usize,
}

impl ShardWriter {
    fn new(dir: &Path, start_seq: u64) -> std::io::Result<Self> {
        let path = dir.join(format!("{}.jsonl", start_seq));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            seq: start_seq,
            file,
            cur_bytes: 0,
        })
    }

    /// Serialize and append one entry as a JSONL line. Rolls to a new shard
    /// when appending the line would exceed [`TARGET_SHARD_BYTES`]. A single
    /// `write!` of one line under `O_APPEND` is atomic on POSIX, so concurrent
    /// writers cannot interleave — here the writer is single-threaded anyway.
    fn emit(&mut self, entry: &CacheEntry) {
        let mut line = match serde_json::to_string(entry) {
            Ok(s) => s,
            Err(e) => {
                warn!("cache: cannot serialize entry: {}", e);
                return;
            }
        };
        line.push('\n');
        let bytes = line.as_bytes();

        if self.cur_bytes + bytes.len() > TARGET_SHARD_BYTES && self.cur_bytes > 0 {
            self.fsync();
            self.seq += 1;
            let path = self.dir.join(format!("{}.jsonl", self.seq));
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    self.file = f;
                    self.cur_bytes = 0;
                }
                Err(e) => {
                    error!("cache: cannot roll shard {}: {}", path.display(), e);
                    return;
                }
            }
        }

        if let Err(e) = self.file.write_all(bytes) {
            error!("cache: write failed: {}", e);
            return;
        }
        self.cur_bytes += bytes.len();
    }

    /// Flush and fsync the current shard file to disk.
    fn fsync(&self) {
        let _ = self.file.sync_all();
    }
}

// ---------------------------------------------------------------------------
// Small utilities
// ---------------------------------------------------------------------------

/// Convert a `SystemTime` to unix seconds (full second granularity).
fn unix_secs(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// fsync a directory (to persist file creation/deletion). Errors are ignored
/// — this is a best-effort durability step on the crash-recovery path.
fn fsync_dir(dir: &Path) {
    if let Ok(f) = FsFile::open(dir) {
        let _ = f.sync_all();
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::ffprobe::Probe;
    use std::path::Path;
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

    /// A stub `Probe` that always fails, exercising the "probe failure leaves
    /// the file uncached" path.
    struct FailingProbe;

    impl Probe for FailingProbe {
        fn probe(&self, file: &WalkFile) -> Result<MediaFeatures, Error> {
            Err(Error::ProbeFailed {
                path: file.path.display().to_string(),
                reason: "stub failure".into(),
            })
        }
    }

    fn walk_file(dir: &Path, name: &str, size: u64, mtime: SystemTime) -> WalkFile {
        let path = dir.join(name);
        std::fs::write(&path, vec![0u8; size as usize]).unwrap();
        // `walk::File::created` carries the mtime explicitly (in real usage it
        // comes from metadata.modified()); entry_hash uses only that field, so
        // we pass `mtime` directly without needing to set the on-disk mtime.
        WalkFile {
            path: crate::path::AbsPath::from_abs_path(&path),
            size,
            created: mtime,
        }
    }

    /// `entry_hash` must be stable for equal (path, mtime, size) and must
    /// differ when any input changes.
    #[test]
    fn entry_hash_is_stable_and_sensitive() {
        let p = Path::new("/abs/path/video.mp4");
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let h1 = entry_hash(p, m, 1000);
        let h2 = entry_hash(p, m, 1000);
        assert_eq!(h1, h2, "same inputs must hash equally");

        // Different path.
        assert_ne!(h1, entry_hash(Path::new("/abs/path/other.mp4"), m, 1000));
        // Different mtime.
        assert_ne!(h1, entry_hash(p, m + Duration::from_secs(1), 1000));
        // Different size.
        assert_ne!(h1, entry_hash(p, m, 2000));

        // Hex SHA-256 is 64 chars.
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A fresh populate against an empty cache probes every file and writes
    /// one entry per file, readable on the next load.
    #[test]
    fn populate_probes_and_persists_missing() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let f1 = walk_file(tmp.path(), "a.mp4", 1_000, m);
        let f2 = walk_file(tmp.path(), "b.mp4", 2_000, m);
        let files = vec![f1.clone(), f2.clone()];

        cache.populate(&files, &StubProbe);

        let loaded = cache.load_all();
        assert_eq!(loaded.len(), 2);
        let keys: Vec<String> = loaded.iter().map(|e| e.key.clone()).collect();
        assert!(keys.contains(&entry_hash(&f1.path, f1.created, f1.size)));
        assert!(keys.contains(&entry_hash(&f2.path, f2.created, f2.size)));
    }

    /// Matched entries get `last_used` refreshed and are NOT re-probed: a
    /// failing probe leaves them intact when the file is already cached.
    #[test]
    fn matched_entries_are_refreshed_not_reprobed() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let f = walk_file(tmp.path(), "a.mp4", 1_000, m);
        let files = vec![f.clone()];

        // First populate with the stub probe seeds the cache.
        cache.populate(&files, &StubProbe);
        let before = cache.load_all();
        assert_eq!(before.len(), 1);
        let old_last_used = before[0].last_used;

        // Advance time and re-run with a probe that would fail every file. The
        // already-cached entry must survive (refreshed) and not be re-probed.
        std::thread::sleep(Duration::from_millis(1100));
        cache.populate(&files, &FailingProbe);
        let after = cache.load_all();
        assert_eq!(after.len(), 1, "matched entry survives a failing probe");
        assert!(
            after[0].last_used >= old_last_used,
            "last_used is refreshed on match"
        );
    }

    /// Unmatched-but-fresh entries survive (e.g. a subset scan); stale entries
    /// past the TTL are dropped.
    #[test]
    fn ttl_expires_unmatched_and_keeps_fresh() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let fresh = walk_file(tmp.path(), "fresh.mp4", 1_000, m);
        let stale = walk_file(tmp.path(), "stale.mp4", 2_000, m);

        // Seed both.
        cache.populate(&[fresh.clone(), stale.clone()], &StubProbe);

        // Manually age the `stale` entry's last_used beyond the TTL by editing
        // the shard on disk directly. (populate itself always sets now; this
        // simulates a file that was unseen for longer than the TTL.)
        let max_seq = cache.max_seq().unwrap();
        let shard = tmp.path().join(format!("{}.jsonl", max_seq));
        let content = std::fs::read_to_string(&shard).unwrap();
        let stale_key = entry_hash(&stale.path, stale.created, stale.size);
        let aged: String = content
            .lines()
            .map(|line| {
                if line.contains(&stale_key) {
                    line.replace(
                        &format!("\"u\":{}", extract_last_used(line)),
                        &format!("\"u\":{}", 1_700_000_000 - 31 * 86_400),
                    )
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&shard, aged + "\n").unwrap();

        // Re-run scanning only `fresh`: stale is unmatched and past TTL → dropped.
        cache.populate(&[fresh.clone()], &StubProbe);
        let loaded = cache.load_all();
        let keys: Vec<String> = loaded.iter().map(|e| e.key.clone()).collect();
        assert!(keys.contains(&entry_hash(&fresh.path, fresh.created, fresh.size)));
        assert!(
            !keys.contains(&stale_key),
            "stale unmatched entry is expired past TTL"
        );
    }

    /// Extracts the integer value of the `"u":N` field from a JSONL line, for
    /// test manipulation of `last_used`.
    fn extract_last_used(line: &str) -> i64 {
        let idx = line.find("\"u\":").expect("u field present");
        let rest = &line[idx + 4..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest[..end].parse().unwrap()
    }

    /// A corrupt shard is discarded on load and its files simply re-probed; it
    /// never aborts the populate pass.
    #[test]
    fn corrupt_shard_is_discarded() {
        let tmp = tempdir().unwrap();
        // Pre-seed a corrupt shard at seq 1.
        std::fs::create_dir_all(tmp.path()).unwrap();
        std::fs::write(
            tmp.path().join("1.jsonl"),
            "this is not json at all\n{also broken\n",
        )
        .unwrap();

        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let f = walk_file(tmp.path(), "a.mp4", 1_000, m);

        // Should not panic; the file gets re-probed and cached.
        cache.populate(&[f.clone()], &StubProbe);
        let loaded = cache.load_all();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].key, entry_hash(&f.path, f.created, f.size));
    }

    /// The old generation is deleted after the new one is written: only the
    /// newest seq(s) remain on disk after a populate.
    #[test]
    fn old_generation_is_deleted() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let f = walk_file(tmp.path(), "a.mp4", 1_000, m);

        cache.populate(&[f.clone()], &StubProbe);
        let seqs_after_first: Vec<u64> = seqs_on_disk(tmp.path());
        let max1 = *seqs_after_first.iter().max().unwrap();

        cache.populate(&[f.clone()], &StubProbe);
        let seqs_after_second: Vec<u64> = seqs_on_disk(tmp.path());
        let max2 = *seqs_after_second.iter().max().unwrap();

        assert!(max2 > max1, "seq grows monotonically across runs");
        assert!(
            seqs_after_second.iter().all(|s| *s > max1),
            "old generation (<= max1) is deleted after rewrite"
        );
    }

    fn seqs_on_disk(dir: &Path) -> Vec<u64> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                p.file_stem()?.to_str()?.parse::<u64>().ok()
            })
            .collect()
    }

    /// Survivors and freshly probed entries share the same new generation.
    #[test]
    fn survivors_and_probes_share_generation() {
        let tmp = tempdir().unwrap();
        let cache = Cache::new(tmp.path().to_path_buf(), 30);
        let m = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let known = walk_file(tmp.path(), "known.mp4", 1_000, m);
        // Seed the cache with `known`.
        cache.populate(&[known.clone()], &StubProbe);

        // Now add a new file alongside the known one. The known entry is a
        // survivor; the new file is probed. Both must end up cached.
        let newf = walk_file(tmp.path(), "new.mp4", 2_000, m);
        cache.populate(&[known.clone(), newf.clone()], &StubProbe);
        let loaded = cache.load_all();
        assert_eq!(loaded.len(), 2);
    }

    /// `MediaFeatures` serde round-trips with the short keys documented in the
    /// design, and tolerates unknown fields / missing optional fields.
    #[test]
    fn media_features_serde_roundtrip() {
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 8_589_934_592,
            video_codec: "h264".into(),
            audio_codec: "ac3".into(),
            duration_secs: 7200.5,
            fps: Some(23.976),
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"w\":1920"));
        assert!(s.contains("\"h\":1080"));
        assert!(s.contains("\"s\":8589934592"));
        assert!(s.contains("\"vc\":\"h264\""));
        assert!(s.contains("\"ac\":\"ac3\""));
        assert!(s.contains("\"d\":7200.5"));
        assert!(s.contains("\"fps\":23.976"));
        let back: MediaFeatures = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);

        // fps omitted -> defaults to None; unknown field tolerated.
        let no_fps =
            "{\"w\":1280,\"h\":720,\"s\":100,\"vc\":\"\",\"ac\":\"\",\"d\":10.0,\"extra\":42}";
        let parsed: MediaFeatures = serde_json::from_str(no_fps).unwrap();
        assert_eq!(parsed.fps, None);
        assert_eq!(parsed.width, 1280);
    }

    /// `CacheEntry` serde uses the short keys and aliases for back-compat.
    #[test]
    fn cache_entry_serde_keys_and_aliases() {
        let e = CacheEntry {
            key: "abc".into(),
            last_used: 123,
            features: MediaFeatures {
                width: 1,
                height: 2,
                file_size: 3,
                video_codec: "x".into(),
                audio_codec: "y".into(),
                duration_secs: 4.0,
                fps: None,
            },
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"k\":\"abc\""));
        assert!(s.contains("\"u\":123"));
        assert!(s.contains("\"f\":"));
        // fps is skip_serializing_if None.
        assert!(!s.contains("\"fps\""));
        let back: CacheEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);

        // Long-form aliases are accepted on read.
        let long =
            r#"{"key":"z","last_used":9,"features":{"w":1,"h":2,"s":3,"vc":"","ac":"","d":4.0}}"#;
        let parsed: CacheEntry = serde_json::from_str(long).unwrap();
        assert_eq!(parsed.key, "z");
        assert_eq!(parsed.last_used, 9);
    }
}
