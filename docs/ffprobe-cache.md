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
  shard_0.jsonl                              # one shard file
  shard_1.jsonl
  ...
  cache.lock                                 # flock during write+delete
```

- `XDG_CACHE_HOME` is resolved via the `dirs` crate (or an equivalent small
  helper); if unset, `$HOME/.cache`.
- Shard filename: `shard_<seq>.jsonl` where `<seq>` is a **monotonically
  increasing sequence number** allocated per shard written (see *Sharding
  rules*). Sequence numbers are *not* content-derived: each rewrite pass
  allocates `max(existing_seq)+1, max(existing_seq)+2, …`, writes the new
  generation, then deletes the old generation — so a crash mid-rewrite leaves
  the previous (lower-seq) generation intact and valid.

## Hashing

The per-file lookup key uses **SHA-256** (the `sha2` crate), hex-encoded.
(Shard filenames are sequence numbers, not hashes — see *Sharding rules*.)

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

A shard is **JSONL** — one `CacheEntry` per line:

```text
{"k":"9f2a3f9c…","u":1751800000,"f":{"w":1920,"h":1080,"ar":"16:9","vc":"h264","ac":"ac3","br":8500000,"d":7200.5,"fps":23.976}}
{"k":"a1b2c3d4…","u":1751800000,"f":{"w":3840,"h":2160,"ar":"16:9","vc":"hevc","ac":"aac","br":15000000,"d":5400.0,"fps":24.0}}
...
```

Why JSONL over a JSON array (settled by benchmark):
- **Within-shard parallel parse**: each line is a self-contained JSON value,
  so a shard's lines can be split and deserialized across rayon threads *in
  addition to* the across-shard parallelism. A single large shard still
  saturates all cores; an array can only be parsed single-threaded.
- **Line-atomic appends**: a single `write()` of one ~184-byte line to a file
  opened `O_APPEND` is atomic on POSIX, so concurrent writers cannot
  interleave/corrupt lines.
- **Exact byte tracking on roll**: appending lines during the write phase
  means the writer knows the real shard size as it goes — no per-entry byte
  estimate is needed to decide when to roll to the next shard (see *Sharding
  rules*).

Note that no file path appears anywhere in the cache — only the opaque `key`
hash, the `last_used` timestamp, and the extracted features.

A shard is rolled to a new file when appending the next line would exceed
`TARGET_SHARD_BYTES` (8 MiB). All non-final shards are therefore ≈8 MiB; the
final shard may be smaller. A library of N files occupies roughly
`ceil(total_bytes / 8 MiB)` shards (≈24 for 1 M files).

## Sharding rules

- **Always rewrite.** Every startup compaction writes a fresh generation of
  shards from scratch and deletes the previous generation. There is no
  in-place append between runs and no content-addressed skip; the write phase
  simply streams JSONL lines into each shard, tracking the exact byte count,
  and rolls to a new file when the next line would exceed `TARGET_SHARD_BYTES`
  (8 MiB). No per-entry byte estimate is used.
- `TARGET_SHARD_BYTES = 8 * 1024 * 1024`. Measured line size with all fields
  present is ~184 bytes, so an 8 MiB shard holds ~41 600 entries; a 1 M-file
  library produces ~24 shards, each comfortably above the HDD seek/transfer
  break-even (~1.3 MB at 150 MB/s × 9 ms) so cold reads are bandwidth-bound,
  not seek-bound.
- **Filenames are monotonic sequence numbers**: `shard_<seq>.jsonl`. At the
  start of the write phase, scan the dir for `max(existing_seq)` and allocate
  `max+1, max+2, …` for the new generation. Sequence numbers are `u64` and
  never reset; a library rewritten daily for decades stays in the low
  thousands.
- After compaction, entries are sorted by `key` before chunking (cheap,
  `O(N log N)`; makes output deterministic, which lets the dirty-flag skip
  below work) and split into consecutive ≤8 MiB chunks.
- **Dirty-flag skip**: the write phase is skipped entirely if no entry changed
  during steps (2)–(3) — no `last_used` refresh, no drop, no new probe. With
  day-granular `last_used`, same-day reruns of a stable library change
  nothing, so the cache is rewritten **at most once per day**, not on every
  invocation. This is the only skip; there is no incremental append-only path.
- Reading is parallel and two-level: each shard file is loaded on a rayon
  thread, and within each shard the lines are split and deserialized in
  parallel (JSONL's self-delimiting property). Even one large shard saturates
  many cores.

## TTL & startup populate flow

TTL applies to **`last_used`**. An entry stays alive as long as its key is
seen among the collected files within the TTL window. Default TTL: **30 days**,
configurable via `--cache-ttl-days`.

On `App::init` (before tokenization/training), the cache runs a single
populate pass with five steps:

```
// (1) LOAD: read every shard file in parallel (one rayon task per file; within
//     each shard, split the JSONL lines and deserialize in parallel).
//     Resilient to failure: a shard that fails to read or parse (I/O error,
//     corruption, format mismatch) is discarded and its files re-probed in
//     step (3). A single bad shard never aborts the populate pass.
//     Dedup by key (keep the entry with the greatest last_used). This is a
//     no-op in normal operation (each key appears once) but recovers a
//     crash mid-rewrite that left an old generation and a partial new
//     generation on disk together.
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

// (4) WRITE: if nothing changed during (2)-(3) (no refresh, no drop, no new
//     probe), skip entirely — dirty = false; with day-granular last_used,
//     same-day reruns skip. Otherwise take an exclusive flock on cache.lock
//     for the write+delete critical section (the generation switch must not
//     race; appends within it are line-atomic under O_APPEND as a bonus).
//     Sort all entries by key, then stream them as JSONL lines into fresh
//     shards, rolling when the next line would exceed TARGET_SHARD_BYTES
//     (8 MiB). Track real bytes — no per-entry estimate. Filenames are
//     monotonic sequence numbers: base = max(existing_seq) + 1.
if !dirty { skip to after (5) }
flock_exclusive(cache_dir.join("cache.lock"))   // held through (5)
entries.sort_by_key(|e| &e.key)
base = max_seq_in_dir(cache_dir) + 1
seq = base
cur = open(format!("shard_{}.jsonl", seq), O_APPEND | O_CREAT)
cur_bytes = 0
for e in &entries {
    line = serialize_jsonl(e)            // includes trailing '\n'
    if cur_bytes + line.len() > TARGET_SHARD_BYTES && cur_bytes > 0 {
        fsync(cur); close(cur); seq += 1
        cur = open(format!("shard_{}.jsonl", seq), O_APPEND | O_CREAT)
        cur_bytes = 0
    }
    write(cur, line)                     // line-atomic under O_APPEND
    cur_bytes += line.len()
}
fsync(cur); close(cur)

// (5) DELETE: still holding the flock, delete every shard_*.jsonl with
//     seq < base — the previous generation. A crash before this point leaves
//     old + partial-new on disk together; step (1)'s dedup-by-key recovers
//     correctly next run. Worst case, delete the cache dir.
delete_shards_with_seq_below(base)
release flock
```

Step numbering maps to the five phases: **load → compact → ffprobe → write →
delete**.

Key properties:
- **Day-granular `last_used`**: `last_used` is rounded down to the start of
  the current unix day (`floor(now / 86400) * 86400`). For a matched file,
  `last_used` is the only field that changes, and it only changes when the day
  rolls. Same-day reruns therefore change no entry → the dirty flag stays
  false → step (4) is skipped. The cache is rewritten **at most once per day**,
  not on every invocation.
- **Matched entries** (file present & same mtime/size) get `last_used` bumped
  to *today* — exactly the "updated during app init if the entry matches the
  collected files" requirement, at day granularity.
- **Unmatched-but-fresh** entries survive (file may be on a disconnected
  volume, or this run scanned a subset). They keep their old `last_used`.
- **Stale** entries (unmatched and `last_used` older than TTL) are dropped.
- **Newly probed entries** (step 3) are merged with survivors and written out
  in step 4, so a single write pass covers both. Shards are ≤8 MiB; the final
  shard may be smaller.
- **Always-rewrite + dirty-flag skip** (step 4): every run that changes
  anything rewrites the whole cache as a fresh shard generation; a run that
  changes nothing writes nothing. Simpler than an incremental append path, and
  fast enough (<1 s for a 1 M-file library on SSD; the rewrite is sequential
  I/O so it scales with bandwidth).
- **Monotonic-seq filenames + crash safety** (steps 4–5): new shards get
  `seq > max(existing)`; the old generation is deleted only after the new one
  is fsynced. A crash before the delete leaves old + partial-new on disk
  together; step (1)'s `dedup-by-key` (keep greatest `last_used`) recovers
  correctly next run. Worst case, delete the cache dir.
- **Concurrency**: the write+delete critical section is serialized by an
  exclusive `flock` on `cache.lock`; appends within it are line-atomic under
  `O_APPEND` as a second layer of safety. Typically only one process runs;
  the lock makes the rare second process safe.

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
- **Shard format = JSONL, sized by exact bytes (8 MiB), named by monotonic
  sequence number, always-rewrite + delete-old.** Settled by benchmark: JSONL's
  self-delimiting lines enable within-shard parallel deserialize (a single
  large shard still saturates many cores — impossible with a JSON array) and
  line-atomic `O_APPEND` concurrent writes; 8 MiB shards keep cold HDD reads
  bandwidth-bound (above the ~1.3 MB seek/transfer break-even) and a 1 M-file
  library to ~24 files; exact byte tracking on roll removes any per-entry size
  estimate; monotonic sequence numbers give crash-safe generation switches
  without content-addressing. The earlier "1000 entries/shard, JSON array,
  content-hash filename" design is superseded. (Settled, not open.)
- **A `MediaFeatures` classifier** (bucketing duration/fps/etc.) is the next
  design step once the cache exists.
