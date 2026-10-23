# ffprobe Cache — Design

A shared, append-only on-disk cache for the media features extracted by
`ffprobe`. It lets Classi-Cine probe each video once and reuse the result across
runs and across machines (probing is the expensive step).

## Goals

- **Amortize ffprobe cost**: probe each (path, mtime, size) once.
- **Shareable**: one file next to the playlist, usable by several machines that
  mount the same library (e.g. over sshfs) at different absolute paths.
- **Resilient**: a torn line, a failed probe, or a cache I/O error is skipped
  and the affected files are probed again or left uncached. The cache never
  aborts a run; deleting the file by hand is a valid reset.
- **Raw on disk**: only raw, non-derivable values are stored. Aspect ratio and
  bitrate are derived by the classifier at read time, and bucketing or
  normalization happens only in memory, so changing a discretization never
  forces a re-probe.

## Location

One file per playlist root:

```
<playlist root>/.classi-cine-ffprobe-cache.jsonl
```

The playlist root is the M3U's parent directory (`playlist.root()`); the name is
`cache::CACHE_FILE_NAME`. Playlists in the same directory share the file.

## Entry schema

JSONL, one `CacheEntry` per line:

```text
{"p":"movies/a.mp4","m":1751800000,"f":{"w":1920,"h":1080,"s":8589934592,"vc":"h264","ac":"ac3","d":7200.5,"fps":23.976}}
```

- `p`: path relative to the playlist root, produced by the same relativization
  used for M3U entries (`AbsPath::to_string` with `RelativeTo(root)`), so the
  cache and the playlist agree. Paths outside the root become `../...`. Because
  paths are relative, machines that mount the library at different absolute
  paths share entries.
- `m`: mtime in whole unix seconds.
- `f`: `MediaFeatures` (see `src/cache.rs`). The file size is stored once, as
  `f.s` (`file_size`).

The key is `CacheKey { path, mtime, size }`, in plaintext. A changed mtime or
size is a different key, so the file is probed again; the old line is simply
never matched again.

The mtime is truncated to whole seconds on purpose. The file is shared over
sshfs, where SFTP v3 transfers mtime as a 32-bit seconds value, while the local
machine sees nanoseconds. Finer precision would never match across machines.

## Populate flow

`Cache::populate(files, probe) -> HashMap<CacheKey, MediaFeatures>`:

1. **Load**: read the whole file, split into lines, parse in parallel (rayon),
   silently skip lines that do not parse (torn or corrupt), and deduplicate by
   key with the last line winning.
2. **Compute missing**: collected files whose `Cache::key_for(path, mtime,
   size)` is not in the map.
3. **Probe** the missing files in parallel (rayon). Results go over an `mpsc`
   channel to a single writer thread.
4. **Append**: the writer appends each entry as one `write_all` of a complete
   line to a file opened with `create(true).append(true)`, and flushes after
   every line so Ctrl-C keeps progress. `sync_all` runs at the end. The file is
   created lazily, only when there is something to append. If the previous
   writer died mid-line (file not ending in a newline), the next append starts
   with a newline so the torn fragment cannot swallow a good entry.

`App::attach_features` looks each walked file up with `key_for`. A file whose
probe failed is absent from the map and keeps the all-zero default; it is
retried next run.

Appends are one complete line per `write_all` under `O_APPEND`, so machines
writing at the same time interleave whole lines. Two machines may both probe
the same file; the duplicate lines resolve by last-wins on the next load.

## No expiry, no compaction

Entries never expire, and the tool never rewrites, compacts, or deletes the
cache file. Machines sharing a playlist may scan different directory subsets,
so an entry that one machine does not see is not necessarily dead. The cost is
a file that only grows (about 170 bytes per probed file version); to shrink it,
delete the file and let the next run rebuild it.

## `move`

`classi-cine move <old.m3u> <new.m3u>`: if the new playlist's root differs from
the old one, the old root's cache is read, each entry's path is rebased
(`old_root.join(p)`, normalized, then made relative to `new_root`), and entries
not already present are appended to the new root's cache file. The old cache
file is left in place, since other playlists in that directory may share it.
Nothing happens when the roots are equal.

## ffprobe integration

The cache is decoupled from ffprobe via the `Probe` trait (`src/ffprobe.rs`), so
tests use stubs:

```rust
pub trait Probe {
    fn probe(&self, file: &walk::File) -> Result<MediaFeatures, Error>;
}
```

`FfprobeProbe` runs `ffprobe -v error -print_format json -show_format
-show_streams <path>` and extracts width/height, codecs, and fps from the first
video/audio streams, and duration from `format.duration` (required; a missing
duration is a probe failure). `file_size` comes from the walk, not ffprobe.
Probing is eager: the tokenizer and classifiers train over the full corpus, so
every missing file is probed before training begins.

## Error handling

A failed probe leaves that file uncached. A cache file that cannot be read yields
an empty cache; one that cannot be opened for append is logged and the run
continues, probing without caching. `populate` never returns an error.
