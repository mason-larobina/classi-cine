# ffprobe Cache — Design

A persistent, sharded on-disk cache for extracted media features produced by
`ffprobe`. The cache lets Classi-Cine avoid re-probing videos across runs
(probing is the expensive operation this whole subsystem exists to amortize).

## Goals

- **Amortize ffprobe cost**: probe each (path, mtime, size) once, reuse across
  runs.
- **Reduce cache size**: store only the features the classifiers actually
  need, with short serde keys and no redundant fields. Anything derivable from
  what is already stored (aspect ratio from width/height, bitrate from size +
  duration) is derived in memory at read time rather than persisted.
- **Cheap startup**: load only what is needed, in parallel.
- **Self-cleaning**: stale entries expire by TTL; the whole cache is rewritten
  as a fresh shard generation on every startup.
- **Resilient to failure**: a corrupt or unparseable shard, a failed ffprobe,
  or a cache I/O error is discarded/skipped and the affected files are simply
  re-probed or left uncached. A single bad file or shard never aborts a run;
  the cache rebuilds itself from the next walk. Worst case, deleting the cache
  dir is a valid, safe reset.
- **Decoupled from classification**: both `EntryMeta` (playlist) and
  `CacheEntry` (ffprobe cache) store **raw** feature values on disk.
  Normalization, bucketing, and discretization occur **only in-memory, inside
  the classifier**, at read time — never persisted. This keeps the
  discretization strategy mutable without forcing a re-probe of the library:
  change a bucket boundary or a normalization scheme and the next run simply
  reads the same raw values and re-buckets them. This is the canonical
  statement of the raw-on-disk principle; the schemas and ffprobe integration
  below all follow from it.

## Non-goals

- This doc does not design the classifiers that *consume* the features.
- No cross-host cache sharing (paths are absolute).
- No content-addressed storage of the video bytes themselves.

## Relationship to existing code

- `walk::File` already collects per-file `path` (`AbsPath`), `size` (`u64`),
  and `created` (`SystemTime`, which is the file's *modified* time). These are
  the inputs to the per-file lookup key (see below), and `size` is also carried
  through into `MediaFeatures` (see *Entry schema*).
- `playlist::EntryMeta` is the closest existing struct in *shape* (serde,
  short keys, tolerant of unknown fields), but its semantics are classification
  (relative path, score, `added`). Reusing it would couple two unrelated
  concerns and force optional fields onto every playlist entry. The cache uses
  a **dedicated `CacheEntry` struct** that mirrors EntryMeta's serde conventions
  rather than reusing the type.
- **`MediaFeatures` is intended for reuse beyond the cache.** It is the struct
  that will eventually be serialized and stored inside `playlist::EntryMeta`,
  so a playlist entry carries its extracted features directly and the
  classifiers can run without touching the ffprobe cache at all.
- The cache lives under the XDG cache directory, distinct from the playlist
  (which lives wherever the user points it).

## Cache directory layout

```
$XDG_CACHE_HOME/classi-cine/ffprobe/        # falls back to ~/.cache/...
  <seq>.jsonl                                # one shard file (e.g. 7.jsonl, 8.jsonl, …;
  ...                                        #   seqs grow monotonically across runs)
```

- `XDG_CACHE_HOME` is resolved via the `dirs` crate (or an equivalent small
  helper); if unset, `$HOME/.cache`.
- Shard filename: `<seq>.jsonl` where `<seq>` is a **monotonically increasing
  sequence number**. A bare integer stem makes the seq trivial to parse off the
  filename (strip the `.jsonl` suffix) — no prefix to strip, no regex needed.
  Each compaction writes the new generation at `max(existing_seq)+1, +2, …` and
  deletes the previous (lower) generation afterward (see *Sharding rules* and
  *populate flow*), so seqs never reset and grow roughly by the shard count per
  run. A library rewritten daily for decades stays well within `u64`; the
  numbers stay small enough to be human-readable in a directory listing.
- Each shard is a single file of JSONL content (one `CacheEntry` per line); the
  `.jsonl` extension matches the content format and keeps the stem a bare
  integer for easy parsing.

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

**The key is the entry's sole identity.** The plaintext path, mtime, and size
are *not* stored on the entry — they are fully bound into the key. Storing them
again would be redundant: a file with a changed mtime, size, or path produces
a different key and simply won't match, so the old entry expires by TTL
without any explicit validation. Not persisting redundant data is also what
keeps each entry small (the cache-size goal above). As an incidental side
effect, the cache files contain no readable file paths; that privacy property
is not the design's purpose, merely a consequence of not storing what is
already recoverable from the key. (The features themselves — resolution,
duration, codec — are the actual cached data and remain in the clear; they are
semi-identifying of a specific title and unavoidable.)

So a changed file (different mtime/size) or a moved/renamed file (different
path) yields a different key and fails to match, expiring the old entry by TTL
— no separate validation fields are needed. The only thing lost is
human-debuggability: you cannot tell which file an entry refers to without
re-hashing a candidate. Acceptable for a cache that is always used alongside a
fresh walk.

## Entry schema

### `MediaFeatures`

Extracted ffprobe features, stored with short serde keys for compactness.
Stable numeric quantities are typed; volatile free-form identifiers (codec
names) are raw strings. Only raw, non-derivable values are stored; see the
Goals for the rationale.

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MediaFeatures {
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
    /// File size in bytes, sourced from walk::File::size (not ffprobe).
    /// Doubles as the FileSizeClassifier input once MediaFeatures is embedded
    /// in playlist::EntryMeta, and is the numerator for derived bitrate.
    #[serde(rename = "s")]
    pub file_size: u64,
    /// Raw ffprobe video codec name, e.g. "h264", "hevc", "vp9".
    #[serde(rename = "vc", default)]
    pub video_codec: String,
    /// Raw ffprobe audio codec name, e.g. "aac", "ac3".
    #[serde(rename = "ac", default)]
    pub audio_codec: String,
    /// Duration in seconds (float). Near-unique per title, so any classifier
    /// MUST discretize (bucket) this before using it as a feature. Treated as
    /// required: a probe that yields no duration is a probe failure, not a
    /// missing field (ffprobe reports duration for any playable file).
    #[serde(rename = "d")]
    pub duration_secs: f64,
    /// Average frame rate (fps). Discretize before use (e.g. 23.976 vs 60).
    /// Optional because ffprobe occasionally omits it; defaults to None.
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
- `width`/`height`/`file_size`/`duration_secs` are always present. Codec names
  default to empty string if ffprobe omits them; `fps` defaults to `None`.
- **Derived, not stored** (per the raw-on-disk principle). The classifier
  derives, at read time:
  - `aspect_ratio` = reduce(`width`:`height`) by GCD (e.g. 1920×1080 → "16:9").
    This matches ffprobe's `display_aspect_ratio` for the overwhelmingly common
    non-anamorphic case; true anamorphic (SAR != DAR) is not captured, which is
    an acceptable tradeoff for the cache-size saving.
  - `bitrate` = `file_size * 8 / duration_secs` (bits/sec). This is the overall
    container bitrate and is what ffprobe's `format.bit_rate` reports anyway, so
    deriving it loses nothing while removing a per-entry field.
- **Bucketing deferred**. Duration and fps are stored raw precisely *because*
  they are near-unique; a `MediaFeatures` classifier will bucket them (e.g.
  duration into `<30m` / `30–90m` / `90–150m` / `>150m`, fps into `film (≈24)` /
  `tv (≈25/30)` / `high (≥50)`). The classifier transforms raw values into
  features at read time; nothing derived or bucketed is persisted.

### `CacheEntry`

One record per file, serialized into a shard. Mirrors the serde conventions of
`playlist::EntryMeta` (short keys, aliases for readability, tolerant of unknown
fields) but is a separate type.

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CacheEntry {
    /// The per-file lookup key (SHA256-hex of path+mtime+size). This is the
    /// entry's sole identity: the plaintext path/mtime/size are NOT stored,
    /// only their hash. Matching is by key alone.
    #[serde(rename = "k", alias = "key")]
    pub key: String,
    /// Unix seconds of the most recent startup at which this entry's key was
    /// present among the collected files. Stored at full second granularity
    /// (the cache is rewritten every startup regardless, so day-rounding would
    /// buy nothing). Drives TTL expiry.
    #[serde(rename = "u", alias = "last_used")]
    pub last_used: i64,
    /// Extracted ffprobe features.
    #[serde(rename = "f", alias = "features")]
    pub features: MediaFeatures,
}
```

There are no `path`/`mtime`/`size` fields: all three are bound into `key` (see
*Hashing*). (`file_size` *is* stored, but inside `MediaFeatures` where it serves
double duty as a classifier input and the bitrate numerator — it is not a
duplicate of the key's size, which is never recoverable from the hash.) The
entry's role is to carry the features for a given key plus the `last_used`
timestamp that drives expiry; nothing else is needed.

### Shard file format

A shard is **JSONL** — one `CacheEntry` per line:

```text
{"k":"9f2a3f9c…","u":1751800000,"f":{"w":1920,"h":1080,"s":8589934592,"vc":"h264","ac":"ac3","d":7200.5,"fps":23.976}}
{"k":"a1b2c3d4…","u":1751800000,"f":{"w":3840,"h":2160,"s":21474836480,"vc":"hevc","ac":"aac","d":5400.0,"fps":24.0}}
...
```

Note that no file path appears anywhere in the cache — only the opaque `key`
hash, the `last_used` timestamp, and the extracted features.

Why JSONL over a JSON array (settled by benchmark):
- **Within-shard parallel parse**: each line is a self-contained JSON value,
  so a shard's lines can be split and deserialized across rayon threads *in
  addition to* the across-shard parallelism. A single large shard still
  saturates all cores; an array can only be parsed single-threaded.
- **Line-atomic appends**: a single `write()` of one ~170-byte line to a file
  opened `O_APPEND` is atomic on POSIX, so concurrent writers cannot
  interleave/corrupt lines.
- **Exact byte tracking on roll**: appending lines during the write phase
  means the writer knows the real shard size as it goes — no per-entry byte
  estimate is needed to decide when to roll to the next shard.

A shard is rolled to a new file when appending the next line would exceed
`TARGET_SHARD_BYTES` (see below). All non-final shards are therefore ≈8 MiB;
the final shard may be smaller.

## Sharding rules

- **Always rewrite, no skip.** Every startup rewrites the cache as a fresh
  generation of shards and deletes the previous generation. There is no
  in-place append between runs and no dirty-flag skip: even a run that changes
  nothing rewrites every shard. The write phase streams JSONL lines into each
  shard, tracking the exact byte count, and rolls to a new file when the next
  line would exceed `TARGET_SHARD_BYTES`. (The dirty-flag skip is intentionally
  deferred: it adds a code path and a correctness condition for a saving that,
  at <1 s per rewrite on SSD, is not yet worth it. It can be added later without
  changing the format.)
- `TARGET_SHARD_BYTES = 8 * 1024 * 1024`. Measured line size with all fields
  present is ~170 bytes, so an 8 MiB shard holds ~49 000 entries; a 1 M-file
  library produces ~21 shards (`ceil(total_bytes / 8 MiB)`), each comfortably
  above the HDD seek/transfer break-even (~1.3 MB at 150 MB/s × 9 ms) so cold
  reads are bandwidth-bound, not seek-bound.
- **Filenames are monotonic per generation**: the new generation is written to
  `<base+1>.jsonl, <base+2>.jsonl, …` where `base = max(existing_seq)` on disk.
  The previous generation (`seq <= base`) is deleted only after the new one is
  fsynced (see *populate flow*), so a crash mid-write leaves the old (lower-seq)
  generation intact and valid — step (1)'s dedup-by-key recovers cleanly next
  run. A bare integer stem is also the easiest filename to parse (see *Cache
  directory layout*).
- Entries are **not sorted before writing.** The previous design sorted by
  `key` to make output deterministic so a dirty-flag skip could detect "nothing
  changed"; with the skip removed, sort order buys nothing, and streaming
  survivors and freshly probed entries straight into the writer is
  incompatible with a global sort. Matching is by key hash, not position, so
  unsorted output is correct.
- Reading is parallel and two-level: each shard file is loaded on a rayon
  thread, and within each shard the lines are split and deserialized in
  parallel (JSONL's self-delimiting property). Even one large shard saturates
  many cores.

## TTL & startup populate flow

TTL applies to **`last_used`**. An entry stays alive as long as its key is seen
among the collected files within the TTL window. Default TTL: **30 days**,
configurable via `--cache-ttl-days`.

On `App::init` (before tokenization/training), the cache runs a single populate
pass with five steps:

```
// (1) LOAD: read every *.jsonl shard in parallel (one rayon task per file;
//     within each shard, split the JSONL lines and deserialize in parallel).
//     Resilient to failure: a shard that fails to read or parse (I/O error,
//     corruption, format mismatch) is discarded and its files re-probed in
//     step (5). A single bad shard never aborts the populate pass.
//     Dedup by key (keep the entry with the greatest last_used). This is a
//     no-op in normal operation (each key appears once) but recovers a
//     crash mid-rewrite that left an old generation and a partial new
//     generation on disk together.
entries = load_all_shards()

// (2) COMPACT: build the set of live keys from this run's collected files,
//     refresh last_used for matched entries, and drop expired ones. Compute
//     the set of missing files (present on disk, no surviving entry) for
//     step (5). No disk I/O here — pure in-memory.
live_keys = { entry_hash(file) : for each collected Walk::File }
now = unix_secs(now)                  // full second granularity
survivors = []
survivor_keys = set()
for entry in entries {
    if entry.key in live_keys {
        entry.last_used = now            // refresh: file present & unchanged
        survivors.push(entry); survivor_keys.insert(entry.key)
    } else if now - entry.last_used < ttl {
        survivors.push(entry)            // unmatched but fresh: keep as-is
                                         // (removable volume / subset scan)
    }
    // else: unmatched and past TTL -> dropped (not pushed)
}
missing = collected_files.filter(|f| entry_hash(f) not in survivor_keys)

// (3) WRITE survivors to a fresh generation. New shards are numbered
//     base+1, base+2, … where base = max(existing_seq). The old generation
//     (seq <= base) is left intact on disk during this write, so a crash here
//     leaves old + partial-new; step (1)'s dedup-by-key recovers. Stream
//     survivors (unsorted) into shards, rolling at TARGET_SHARD_BYTES by exact
//     byte count. Keep the top shard handle open (fsynced but not closed) so
//     step (5) can append to it. There is no lock file: Classi-Cine is a
//     single-process tool, so the write+delete critical section is implicitly
//     serialized; a concurrent second process would simply produce an extra
//     generation and lose it to the next run's dedup, which is acceptable.
base = max_seq_in_dir(cache_dir)
seq = base + 1
cur = open(format!("{}.jsonl", seq), O_APPEND | O_CREAT)
cur_bytes = 0
fn emit(entry) {
    line = serialize_jsonl(entry)            // includes trailing '\n'
    if cur_bytes + line.len() > TARGET_SHARD_BYTES && cur_bytes > 0 {
        fsync(cur); close(cur); seq += 1
        cur = open(format!("{}.jsonl", seq), O_APPEND | O_CREAT)
        cur_bytes = 0
    }
    write(cur, line)                         // line-atomic under O_APPEND
    cur_bytes += line.len()
}
for e in &survivors { emit(e) }              // existing entries, unsorted
fsync(cur)                                   // persisted; handle stays open for (5)

// (4) DELETE the old generation (sync): now that the new generation is
//     persisted, delete every shard with seq <= base and fsync the directory.
//     This is the delete-before-probe boundary: the old generation is gone
//     before any ffprobe runs in step (5).
for s in seqs_where(s <= base) {
    remove_file(format!("{}.jsonl", s))
}
fsync_dir(cache_dir)

// (5) PROBE + WRITE missing (streamed): probe the missing files in parallel
//     (bounded rayon) and append each result directly to the shard writer —
//     no in-memory accumulation of new entries. Appends go to the top shard
//     left open from step (3), rolling to base+k+1, … as it fills. A crash
//     here leaves the new generation with the survivors but possibly missing
//     some probed entries; those files simply have no entry and re-probe
//     next run.
for f in missing.par_iter() {              // (1)
    match probe(f) {
        Ok(features) => emit(CacheEntry { key: entry_hash(f), last_used: now, features }),
        Err(_) => { /* log and skip; retried next run */ }
    }
}
fsync(cur); close(cur)
fsync_dir(cache_dir)
```

(1) In practice the probe results are fed back to the single owning writer via
a channel so that shard rolling and byte tracking stay on one thread; the
`par_iter` over `missing` supplies the work and the writer drains it. The
`O_APPEND` line-atomicity keeps appends safe against the writer thread alone
(which is all that runs here).

Step numbering maps to the five phases: **load → compact → write-survivors →
delete → probe+write-missing**. The compaction write (step 3) and the
missing-entries write (step 5) are deliberately separate: survivors are the
rewrite of existing data and go to a fresh, crash-safe higher-seq generation
*before* the old one is deleted; the missing entries are *new* ffprobe data
and are appended to that same new generation *after* the delete, streamed
straight from the probe to the writer with no intermediate buffer.

Key properties:
- **Full-second `last_used`**: `last_used` is the unix second of the run
  (`now`), not day-rounded. Because the cache is rewritten every startup
  regardless, day-granularity would save no writes — it would only lose
  precision in the TTL comparison. TTL is `--cache-ttl-days * 86400` seconds.
- **Matched entries** (file present & same mtime/size) get `last_used` bumped
  to *now* — exactly the "updated during app init if the entry matches the
  collected files" requirement.
- **Unmatched-but-fresh** entries survive (file may be on a disconnected
  volume, or this run scanned a subset). They keep their old `last_used`.
- **Stale** entries (unmatched and `now - last_used >= ttl`) are dropped.
- **Newly probed entries** (step 5) are appended directly to the new
  generation shards as they are produced — no intermediate in-memory buffer.
  They share the same generation as the survivors written in step (3); a single
  critical section covers the survivor write, the old-generation delete, and
  the missing-entries write.
- **Crash safety**: the new generation is written to `base+1, …` and fsynced
  *before* the old generation (`<= base`) is deleted (step 3 → step 4), so a
  crash during the survivor write leaves the intact old generation plus a
  partial new one — step (1)'s dedup-by-key recovers next run. The delete
  happens before any ffprobe runs, so a crash or probe failure in step (5)
  leaves the new generation with survivors but missing some probed entries;
  those files re-probe next run. The universal fallback for any corrupted state
  is to delete the cache dir by hand.

## ffprobe integration

Step (5) of the populate flow calls a `Probe` implementation.

### `Probe` trait

The cache is decoupled from ffprobe via a trait so the cache logic can be tested
with a stub and the real backend swapped later. The trait takes a `walk::File`
so the impl can source `file_size` from the already-collected stat rather than
re-statting or asking ffprobe for it.

```rust
pub trait Probe {
    /// Probe a single file and return its extracted features.
    /// Returns an error if ffprobe fails or the output is unusable.
    fn probe(&self, file: &walk::File) -> Result<MediaFeatures, Error>;
}
```

### `FfprobeProbe` implementation (shells out to the binary)

```rust
pub struct FfprobeProbe;

impl Probe for FfprobeProbe {
    fn probe(&self, file: &walk::File) -> Result<MediaFeatures, Error> {
        // ffprobe -v error -print_format json -show_format -show_streams <path>
        let output = std::process::Command::new("ffprobe")
            .args(["-v","error","-print_format","json","-show_format","-show_streams"])
            .arg(file.path.as_ref())
            .output()?;
        if !output.status.success() {
            return Err(Error::ProbeFailed { path: file.path.to_string_lossy().into(),
                                            reason: String::from_utf8_lossy(&output.stderr).into() });
        }
        let json: FfprobeJson = serde_json::from_slice(&output.stdout)?;
        Ok(MediaFeatures::from_ffprobe(&json, file.size))
    }
}
```

Field extraction from ffprobe JSON (`FfprobeJson`) into `MediaFeatures::from_ffprobe`:
- `width`/`height`: from the first video stream (`codec_type == "video"`).
- `file_size`: from `walk::File::size` (passed in, not read from ffprobe).
- `video_codec`/`audio_codec`: `codec_name` of the first video / audio stream.
- `duration_secs`: `format.duration` (seconds, float string). **Required** — if
  absent, the probe returns an error (treated as a probe failure); ffprobe
  reports duration for any playable file.
- `fps`: `avg_frame_rate` of the video stream, evaluated to a float
  (`num/den`), if present; `None` if omitted.

Unreliable/missing optional fields become `None`/empty rather than failing the
whole probe; only a total ffprobe failure, a missing video stream, or a missing
duration is an error. `aspect_ratio` (from width/height via GCD) and `bitrate`
(`file_size * 8 / duration_secs`) are derived by the classifier at read time,
not extracted here.

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
— its files re-probe in step (5) and the shard is rebuilt on the next write
pass. The populate flow as a whole never returns an error: a totally unreadable
cache dir degrades to an empty cache and a full re-probe, which is the correct
recovery. (Deleting the cache dir by hand is an equally valid reset.)

## Module layout (proposed)

```
src/
  cache.rs        // CacheEntry, MediaFeatures, Cache, shard load/save/compact
  ffprobe.rs      // Probe trait, FfprobeProbe impl, ffprobe JSON structs
```

`cache.rs` depends only on `sha2`, `serde`, `serde_json`, `rayon`, and
`crate::path`/`crate::walk`/`crate::Error`. `ffprobe.rs` depends on `cache.rs`
and the `ffprobe` binary at runtime.

## Settled decisions & future work

- **Probe scope**: probing must be **eager** (all collected files up front), not
  lazy. The tokenizer (`PairTokenizer`) and the classifiers (Naive Bayes ngram
  features) are trained over the full corpus before classification, and any
  future `MediaFeatures` classifier needs all feature values present to compute
  frequent features / normalization statistics. A lazy probe would leave the
  tokenizer and classifiers under-trained on the undiscovered features. Step (5)
  therefore probes every missing file before tokenization/training begins.
- **Writes are fully persisted before classification**: steps (3) and (5) are
  fsynced (shards and directory) before control returns to the app's
  tokenization/training phase. By the time tokens are computed and classifiers
  are trained, the cache on disk is consistent with the in-memory feature set.
  There is no deferred write-on-shutdown and no "persist next startup" path —
  classification never observes a cache that is out of sync with disk.
- **Shard format settled by benchmark**: JSONL (not a JSON array), 8 MiB shards
  sized by exact bytes, named `<seq>.jsonl` with monotonic per-generation seqs
  (`base+1, base+2, …`), always-rewrite with old-generation deleted after fsync,
  no lock file, no dirty-flag skip. JSONL's self-delimiting lines give
  within-shard parallel deserialize and line-atomic `O_APPEND` writes; the rest
  follows from the rules above. The earlier "1000 entries/shard, JSON array,
  content-hash filename" design is superseded.
- **Future work**: a `MediaFeatures` classifier (deriving aspect ratio and
  bitrate, bucketing duration/fps/etc.) is the next design step once the cache
  exists, as is embedding `MediaFeatures` into `playlist::EntryMeta`.
