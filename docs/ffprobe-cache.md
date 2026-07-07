# ffprobe Cache — Design

A persistent, sharded on-disk cache for extracted media features produced by
`ffprobe`. The cache lets Classi-Cine avoid re-probing videos across runs
(probing is the expensive operation this whole subsystem exists to amortize).

## Goals

- **Amortize ffprobe cost**: probe each (path, mtime, size) once, reuse across
  runs.
- **Cheap startup**: load only what is needed, in parallel.
- **Self-cleaning**: stale entries expire by TTL; surviving entries are
  compacted back into fresh shards on every startup.
- **Resilient to failure**: a corrupt or unparseable shard, a failed ffprobe,
  or a cache I/O error is discarded/skipped and the affected files are simply
  re-probed or left uncached. A single bad file or shard never aborts a run;
  the cache rebuilds itself from the next walk. Worst case, deleting the cache
  dir is a valid, safe reset.
- **Decoupled from classification**: both `EntryMeta` (playlist) and
  `CacheEntry` (ffprobe cache) store **raw** feature values on disk.
  Normalization, bucketing, and discretization occur **only in-memory, inside
  the classifier**, at read time — never persisted. This keeps the discretization
  strategy mutable without forcing a re-probe of the library: change a bucket
  boundary or a normalization scheme and the next run simply reads the same raw
  values and re-buckets them. Persisting derived/bucketed values would freeze
  the transform at write time and couple the cache to a specific classifier
  version.

## Non-goals

- This doc does not design the classifiers that *consume* the features.
- No cross-host cache sharing (paths are absolute).
- No content-addressed storage of the video bytes themselves.

## Relationship to existing code

- `walk::File` already collects per-file `path` (`AbsPath`), `size` (`u64`),
  and `created` (`SystemTime`, which is the file's *modified* time). These are
  the inputs to the per-file lookup key (see below).
- `playlist::EntryMeta` is the closest existing struct in *shape* (serde,
  short keys, tolerant of unknown fields), but its semantics are classification
  (relative path, score, `added`). Reusing it would couple two unrelated
  concerns and force optional fields onto every playlist entry. The cache uses
  a **dedicated `CacheEntry` struct** that mirrors EntryMeta's serde conventions
  rather than reusing the type.
- The cache lives under the XDG cache directory, distinct from the playlist
  (which lives wherever the user points it).

## Cache directory layout

```
$XDG_CACHE_HOME/classi-cine/ffprobe/        # falls back to ~/.cache/...
  shard_<64hex>.json                         # one shard file
  shard_<64hex>.json
  ...
```

- `XDG_CACHE_HOME` is resolved via the `dirs` crate (or an equivalent small
  helper); if unset, `$HOME/.cache`.
- Shard filename: `shard_<sha256-hex>.json` where the digest is the **content
  hash** computed over the shard's canonical serialized entry array (see
  *Hashing*). This makes a shard's filename a verifiable function of its
  contents, so renames, dedup, and partial corruption are detectable.

## Hashing

Both hashes use **SHA-256** (the `sha2` crate), hex-encoded.

### Per-file lookup key — `entry_hash`

A stable identifier for "this exact version of this file", cheap to compute
without reading file contents:

```
entry_hash = SHA256( canonical_abs_path || ":" || mtime_secs || ":" || size )
```

- `canonical_abs_path`: the normalized absolute path string (same
  normalization as `AbsPath`, so `..`/`.` resolved). This is the path the app
  processes internally.
- `mtime_secs`: file modification time in unix seconds (`walk::File::created`,
  already a `SystemTime`, converted via `duration_since(UNIX_EPOCH)`).
- `size`: file size in bytes (`walk::File::size`).

Rationale: mtime + size is sufficient to invalidate the cache entry whenever the
file is rewritten (size changes and/or mtime changes). It avoids the cost of
hashing gigabytes of video bytes on every run. The path is included so two
different files with identical mtime/size don't collide.

**The key is the entry's sole identity.** The plaintext path, mtime and size
are *not* stored on the entry — they are fully bound into the key. Because the
key is a one-way SHA-256 hash, the cache files contain no readable file paths
(privacy-preserving): an attacker who reads the cache dir sees only hashes and
media features, not your directory layout or filenames. (The features
themselves — resolution, duration, bitrate — can still be semi-identifying of a
specific title; that is the actual cached data and unavoidable.)

Consequence for matching: a changed file (different mtime/size) or a
moved/renamed file (different path) yields a *different* key, so the old entry
simply fails to match and expires by TTL — no separate validation fields are
needed. The only thing lost is human-debuggability: you cannot tell which file
an entry refers to without re-hashing a candidate. Acceptable for a cache that
is always used alongside a fresh walk.

### Shard content hash — `shard_hash`

```
shard_hash = SHA256( canonical_serialized_entry_array )
```

- Filename: `shard_` + hex(`shard_hash`) + `.json`.

## Entry schema

### `MediaFeatures`

Extracted ffprobe features, stored with short serde keys for compactness.
Stable numeric quantities are typed; volatile free-form identifiers (codec
names) are raw strings.

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaFeatures {
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
    /// Reduced ratio string, e.g. "16:9", "4:3". Stored as a string because
    /// the reduction is ffprobe's; consumers may re-derive from w/h.
    #[serde(rename = "ar", default)]
    pub aspect_ratio: String,
    /// Raw ffprobe video codec name, e.g. "h264", "hevc", "vp9".
    #[serde(rename = "vc", default)]
    pub video_codec: String,
    /// Raw ffprobe audio codec name, e.g. "aac", "ac3".
    #[serde(rename = "ac", default)]
    pub audio_codec: String,
    /// Overall bitrate in bits/sec, when ffprobe reports it.
    #[serde(rename = "br", default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u64>,
    /// Duration in seconds (float). Near-unique per title, so any classifier
    /// MUST discretize (bucket) this before using it as a feature.
    #[serde(rename = "d", default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    /// Average frame rate (fps). Discretize before use (e.g. 23.976 vs 60).
    #[serde(rename = "fps", default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}
```

Design notes:
- `default` on optional/volatile fields keeps older cache files readable when
  new fields are added later (forward/backward compatible), mirroring
  `EntryMeta`'s tolerance of unknown fields.
- **Schema evolution policy**: no migration step, ever. If a shard file fails
  to parse (corruption or incompatible format), discard the whole shard — its
  files will simply be re-probed. If an entry parses but is missing fields,
  accept the entry with those fields empty/`None` (this is what `default`
  provides). Fields should be added or removed only rarely; when they are, the
  `default`/unknown-field tolerance above is the entire compatibility story.
- `width`/`height` are always present (a probe that yields no resolution is a
  probe failure, not a missing field). Codec names default to empty string if
  ffprobe omits them.
- **Raw values, bucketing deferred.** Duration and fps are stored raw
  precisely *because* they are near-unique; a `MediaFeatures` classifier will
  bucket them (e.g. duration into `<30m` / `30–90m` / `90–150m` / `>150m`,
  fps into `film (≈24)` / `tv (≈25/30)` / `high (≥50)`). This is a specific
  instance of the raw-on-disk principle stated in the Goals: neither
  `MediaFeatures` nor `CacheEntry` persist derived/bucketed values; only the
  in-memory classifier transforms raw values into features.

### `CacheEntry`

One record per file, serialized into a shard. Mirrors the serde conventions of
`playlist::EntryMeta` (short keys, aliases for readability, tolerant of unknown
fields) but is a separate type.

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// The per-file lookup key (SHA256-hex of path+mtime+size). This is the
    /// entry's sole identity: the plaintext path/mtime/size are NOT stored,
    /// only their hash (privacy-preserving). Matching is by key alone.
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
```

Why no `path`/`mtime`/`size` fields: all three are bound into `key`. Storing
them again would (a) leak the plaintext path — the very privacy concern this
design avoids — and (b) be redundant: a file with a changed mtime, size, or
path produces a different key and simply won't match, so the old entry expires
by TTL without any explicit validation. The entry's role is to carry the
*features* for a given key plus the `last_used` timestamp that drives expiry;
nothing else is needed.

### Shard file format

A shard is a JSON array of `CacheEntry`:

```json
[
  {"k":"9f2a3f9c…","u":1751800000,"f":{"w":1920,"h":1080,"ar":"16:9","vc":"h264","ac":"ac3","br":8500000,"d":7200.5,"fps":23.976}},
  ...
]
```

Note that no file path appears anywhere in the cache — only the opaque `key`
hash, two timestamps, and the extracted features.

Maximum 1000 entries per shard. A library of N files therefore occupies
`ceil(N/1000)` shard files.

## Sharding rules

- Each shard holds **at most 1000 entries**.
- After compaction, entries are sorted by `key` and split into consecutive
  chunks of ≤1000. Each chunk's filename is the content hash of its serialized,
  sorted array.
- Shards are rewritten in full (no in-place append) during the startup
  compaction pass; this keeps shard files balanced and filenames correct.
- Reading is parallel: each shard file is loaded and deserialized on a rayon
  thread (rayon is already a dependency). With 1000 entries/shard, even a
  100k-file library is ~100 files, each cheap to parse.

## TTL & startup populate flow

TTL applies to **`last_used`**. An entry stays alive as long as its key is
seen among the collected files within the TTL window. Default TTL: **30 days**,
configurable via `--cache-ttl-days`.

On `App::init` (before tokenization/training), the cache runs a single
populate pass with five steps:

```
// (1) LOAD: read every shard file in parallel, deserialize to Vec<CacheEntry>.
//     Resilient to failure: a shard that fails to read or parse (I/O error,
//     corruption, format mismatch) is discarded and its files re-probed in
//     step (3). A single bad shard never aborts the populate pass.
//     Dedup by key (on collision keep the newest last_used).
entries = load_all_shards()

// (2) COMPACT: build the set of live keys from this run's collected files,
//     then drop expired entries in memory.
live_keys = { entry_hash(file) : for each collected Walk::File }
today = unix_day(now)              // floor(now / 86400) * 86400
for entry in &mut entries {
    if entry.key in live_keys {
        entry.last_used = today       // refresh: file present & unchanged
    } else if now - entry.last_used >= ttl {
        entry.remove()               // expired: not seen this run, past TTL
    }
    // else: unmatched but fresh -> kept as-is (removable volume / subset scan)
}

// (3) FFPROBE: for every collected file whose key has no surviving entry,
//     shell out to ffprobe (bounded rayon concurrency) and build new entries.
missing = collected_files.filter(|f| !entries.contains_key(entry_hash(f)))
for f in missing.par_iter() {
    features = probe(f)              // -> MediaFeatures; errors logged, skipped
    entries.push(CacheEntry { key: entry_hash(f), last_used: today, features })
}

// (4) WRITE: sort all entries (survivors + newly probed) by key, split into
//     consecutive chunks of <=1000, and write each chunk as a fresh shard.
entries.sort_by_key(|e| &e.key)
for chunk in entries.chunks(1000) {
    hash = sha256(serialize(chunk))
    path = cache_dir.join(format!("shard_{}.json", hash))
    if !path.exists() {                // content-addressed: skip identical shards
        atomic_write(path, serialize(chunk))   // temp file + rename
    }
}

// (5) DELETE: remove every old shard_*.json whose filename is not in the
//     new set (these held only expired/removed entries).
delete_obsolete_shards(new_filenames)
```

Step numbering maps to the five phases: **load → compact → ffprobe → write →
delete**.

Key properties:
- **Day-granular `last_used`**: `last_used` is rounded down to the start of
  the current unix day (`floor(now / 86400) * 86400`). Because the shard
  filename is the hash of the serialized entries, and `last_used` is the only
  field that changes for a matched file, runs within the same day produce
  identical shard bytes, the same content hash, and are skipped in step 4.
  Most shards therefore rewrite at most once per day, not on every invocation —
  turning a daily-rewrite cost into a daily one. (Newly probed entries are also
  stamped with `today`, so they integrate into the same scheme.)
- **Matched entries** (file present & same mtime/size) get `last_used` bumped
  to *today* — exactly the "updated during app init if the entry matches the
  collected files" requirement, at day granularity.
- **Unmatched-but-fresh** entries survive (file may be on a disconnected
  volume, or this run scanned a subset). They keep their old `last_used`.
- **Stale** entries (unmatched and `last_used` older than TTL) are dropped.
- **Newly probed entries** (step 3) are merged with survivors and written out
  in step 4, so a single write pass covers both. Each shard is at most 1000
  entries; the final chunk is the partial remainder.
- **Content-addressed writes** (step 4): because a shard's filename is the hash
  of its contents, a shard whose exact byte content already exists on disk is
  skipped — no rewrite. Combined with day-granular `last_used`, this means a
  stable library touched repeatedly on the same day rewrites nothing.
- **Crash safety** (step 5): old shards are deleted only *after* new ones are
  written, so a crash mid-populate loses nothing — old shards remain valid and
  the next run recompacts.

## ffprobe integration

Step (3) of the populate flow calls a `Probe` implementation.

### `Probe` trait

The cache is decoupled from ffprobe via a trait so the cache logic can be
tested with a stub and the real backend swapped later.

```rust
pub trait Probe {
    /// Probe a single file and return its extracted features.
    /// Returns an error if ffprobe fails or the output is unusable.
    fn probe(&self, path: &AbsPath) -> Result<MediaFeatures, Error>;
}
```

### `FfprobeProbe` implementation (shells out to the binary)

```rust
pub struct FfprobeProbe;

impl Probe for FfprobeProbe {
    fn probe(&self, path: &AbsPath) -> Result<MediaFeatures, Error> {
        // ffprobe -v error -print_format json -show_format -show_streams <path>
        let output = std::process::Command::new("ffprobe")
            .args(["-v","error","-print_format","json","-show_format","-show_streams"])
            .arg(path.as_ref())
            .output()?;
        if !output.status.success() {
            return Err(Error::ProbeFailed(String::from_utf8_lossy(&output.stderr).into()));
        }
        let json: FfprobeJson = serde_json::from_slice(&output.stdout)?;
        Ok(MediaFeatures::from_ffprobe(&json))
    }
}
```

Field extraction from ffprobe JSON (`FfprobeJson`):
- `width`/`height`: from the first video stream (`codec_type == "video"`).
- `aspect_ratio`: ffprobe's `display_aspect_ratio`, reduced (ffprobe already
  reduces, e.g. "16:9"); fallback to computing from w/h and reducing by GCD.
- `video_codec`/`audio_codec`: `codec_name` of the first video / audio stream.
- `bitrate`: `format.bit_rate` (overall), if present.
- `duration_secs`: `format.duration` (seconds, float string), if present.
- `fps`: `avg_frame_rate` of the video stream, evaluated to a float
  (`num/den`), if present.

Unreliable/missing fields become `None`/empty rather than failing the whole
probe; only a total ffprobe failure or no-video-stream result is an error.

## CLI flag

Add to the shared args (visible to all subcommands that scan files):

```
--cache-ttl-days <DAYS>     # default: 30
```

Stored in `CommonArgs`, threaded into the cache compaction call. A value of `0`
disables TTL entirely (never expire), which is useful for cold, stable
libraries; there is no "force-expire-everything" flag (simply delete the cache
dir).

## Error handling

Add to the `Error` enum in `main.rs`:

```rust
#[error("ffprobe failed for {path}: {reason}")]
ProbeFailed { path: String, reason: String },

#[error("Cache error: {0}")]
Cache(String),
```

A failed probe for one file does **not** abort the run: the file is simply left
without a cache entry and may be retried next run. Per-shard failures (I/O
error, unparseable JSON, schema mismatch) are logged and the shard is discarded
— its files re-probe in step (3) and the shard is rebuilt on the next write
pass. The populate flow as a whole never returns an error: a totally
unreadable cache dir degrades to an empty cache and a full re-probe, which is
the correct recovery. (Deleting the cache dir by hand is an equally valid
reset.)

## Module layout (proposed)

```
src/
  cache.rs        // CacheEntry, MediaFeatures, Cache, shard load/save/compact
  ffprobe.rs      // Probe trait, FfprobeProbe impl, ffprobe JSON structs
```

`cache.rs` depends only on `sha2`, `serde`, `serde_json`, `rayon`, `chrono`,
and `crate::path`/`crate::Error`. `ffprobe.rs` depends on `cache.rs` and the
`ffprobe` binary at runtime.

## Open questions / future work

- **Probe scope**: probing must be **eager** (all collected files up front),
  not lazy. The tokenizer (`PairTokenizer`) and the classifiers (Naive Bayes
  ngram features) are trained over the full corpus before classification, and
  any future `MediaFeatures` classifier needs all feature values present to
  compute frequent features / normalization statistics. A lazy probe that only
  populated features for files about to be scored would leave the tokenizer
  and classifiers under-trained on the undiscovered features. This is settled
  rather than open: step (3) of the populate flow probes every missing file
  before tokenization/training begins.
- **Writes are fully persisted before classification**: the only in-memory
  mutation of the cache is the entry list during step (3) ffprobe. Step (4)
  writes all entries (survivors + newly probed) to disk, and step (5) deletes
  obsolete shards, *before* control returns to the app's tokenization/training
  phase. By the time tokens are computed and classifiers are trained, the cache
  on disk is consistent with the in-memory feature set. There is no deferred
  write-on-shutdown and no "persist next startup" path — classification never
  observes a cache that is out of sync with disk. (Settled, not open.)
- **A `MediaFeatures` classifier** (bucketing duration/fps/etc.) is the next
  design step once the cache exists.
