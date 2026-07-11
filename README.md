![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Autocomplete, but for your video library.** Classi-Cine watches which videos you keep and which you skip, then predicts what you'll want next — so you build the perfect playlist in a few clicks instead of hunting for an hour.

## Why

A big video collection is only useful if you can find the right thing in it — and usually you can't: scrolling hundreds of files, forgetting which episodes were good, no quick "more like this," hand-built playlists that take longer than watching them.

## How it works

Classi-Cine runs a fast round of accept/reject to home in on what you want:

1. **Point it at your video directories** — it discovers your whole collection.
1. **It suggests, you decide** — preview each pick in VLC; **stop** means *more like this*, **pause** means *less like this*.
1. **Suggestions sharpen as you go** — every decision teaches it your taste.
1. **Walk away with a finished playlist** — minutes, not an afternoon.

Like text autocomplete learns from what you type, Classi-Cine learns from what you keep. It reads file names, folder structure, sizes, and ages — and, via ffprobe, codec, resolution, aspect ratio, duration, bitrate, and frame rate — then scores every video with several classifiers and re-ranks in real time. The next suggestion is always its best guess.

Great for large media collections, content creators curating footage, anyone with 100+ videos, or building themed playlists.

## Quick start

```bash
# Install
cargo install classi-cine

# Build a playlist — the TUI ranks candidates by predicted relevance
classi-cine build my-playlist.m3u ~/Videos ~/Movies
```

The TUI shows AI predictions ranked by relevance. Press **stop** (s) to keep and add it, **pause** (space) to skip. Suggestions get smarter with each decision; quit with **q** or **Esc**.

## Recipe: cull a folder of clips

A favorite use is the inverse of building a "keep" list: quickly triaging a messy folder and deleting the dross. Flip the meaning of the labels — Classi-Cine marks stopped videos as positive, so let **positive mean "delete."** Press stop (s) on clips you want gone, pause (space) on the keepers. As you classify, the playlist learns the patterns of the unwanted clips and surfaces more of them, so culling gets faster the longer you go.

```bash
# Classify clips into delete.m3u: stop (s) = delete, pause (space) = keep
classi-cine build delete.m3u ~/Clips

# Review the delete list before pulling the trigger
classi-cine list-positive delete.m3u

# Delete everything marked for deletion
# (-d '\n' keeps filenames with spaces intact)
classi-cine list-positive delete.m3u | xargs -d '\n' rm -v
```

## Installation

- **VLC** — required for `build` (playback and feedback).
- **ffprobe** — recommended; Classi-Cine extracts codec, resolution, duration, bitrate, and fps to classify on. Probing is cached and failures are skipped silently, so it works without it but learns less.

Then, with Rust and Cargo ([rustup](https://rustup.rs) if needed):

```bash
cargo install classi-cine            # from crates.io
# …or from source:
git clone https://github.com/mason-larobina/classi-cine.git
cd classi-cine && cargo install --path=.
```

## Usage

```text
$ classi-cine --help
Usage: classi-cine [OPTIONS] <COMMAND>

Commands:
  build          Build playlist through interactive classification
  score          Score files using trained classifiers without interactive classification
  list-positive  List positively classified files
  list-negative  List negatively classified files
  move           Move playlist to a new location and rebase paths
  reconcile      Reconcile playlist with disk: drop bare lines for deleted files and re-add them for files that reappeared
  help           Print this message or the help of the given subcommand(s)

Options:
      --log-level <LOG_LEVEL>  [default: info]
      --log-file <LOG_FILE>    Write log output to this file. When set, logs always go to the file, even while the interactive TUI is running (which suppresses stderr logs)
  -h, --help                   Print help
```

### build

```text
$ classi-cine build --help
Build playlist through interactive classification

Usage: classi-cine build [OPTIONS] <PLAYLIST> [DIRS]...

Arguments:
  <PLAYLIST>  M3U playlist file
  [DIRS]...   Directories to scan for video files

Options:
      --exclude <GLOB>
          Glob patterns of files and directories to exclude from the walk, matched against the normalized absolute path. A directory that matches is pruned entirely (its subtree is never descended into). Uses gitignore-flavored rules: a pattern with **no slash** is matched against the file/directory *name* (so `*.tmp` excludes any `.tmp` file at any depth and `sample` prunes any directory *or* file named `sample` anywhere); a pattern **with a slash** is matched against the full absolute path (so `**/trailers/**` excludes files under any `trailers` dir and `/abs/dir/**` anchors to a specific path). `globset` with `literal_separator(true)` applies: `*` matches a single path component and `**` spans any number of them. May be given multiple times; all patterns are OR-ed together
      --video-exts <VIDEO_EXTS>
          Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>
          Maximum contiguous window size for ngram features. Set to 0 to disable windows and rely solely on --combinations [default: 5]
      --combinations <COMBINATIONS>
          Generate orderless combinations of up to k tokens (default pairs) as ngram features. Independent of --windows, so --windows=0 leaves only combinations; set --combinations=0 to disable them entirely [default: 2]
      --file-size-bias <FILE_SIZE_BIAS>
          Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET>
          Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>
          Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET>
          Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>
          Bias scoring based on file age (log base, > 1.0). Negative reverses bias (older files get higher score)
      --file-age-offset <FILE_AGE_OFFSET>
          Offset to add to file age in seconds before log scaling [default: 86400]
      --cache-ttl-days <CACHE_TTL_DAYS>
          Cache TTL in days for the ffprobe feature cache. Entries whose key is not seen among the collected files for this long are expired. 0 disables expiry entirely (useful for cold, stable libraries). To force expire everything, delete the cache directory [default: 30]
      --features-combinations <FEATURES_COMBINATIONS>
          Orderless cross-feature combination order. Feature tokens (categorical singletons + per-continuous-feature neighbor singletons) are fed to `Ngrams::combinations` at this order, producing cross-feature ngrams like `{video_codec:h264, duration:21}`. `0` disables feature ngrams entirely (the feature `combinations` call is skipped). Independent of `--combinations`. See `docs/media-features-classifier.md` [default: 2]
      --features-smoothing <FEATURES_SMOOTHING>
          Neighbor smoothing half-width for continuous buckets. A value in bucket `i` also emits its immediate neighbors `[i-w, i+w]` (clamped to `>= 0`), so adjacent buckets share signal through overlapping singletons. `0` disables smoothing (plain 1-bucket singletons) [default: 1]
      --features-bucket-base <FEATURES_BUCKET_BASE>
          Geometric bucket base for `duration` / `filesize` / `bitrate` (> 1.0). `bucket(v) = floor(log_base(max(v, 1.0)))`. Power-of-2 bases are too coarse for media; 1.5 yields ~22 duration buckets across 1s–4h [default: 1.5]
      --features-fps-base <FEATURES_FPS_BASE>
          Geometric bucket base for `fps` (> 1.0), separate from `--features-bucket-base` because the fps range is narrow (~10–120) and clustered at standard rates. 1.1 keeps NTSC/PAL partners (23.976/24, 29.97/30, 59.94/60) in single buckets while separating adjacent groups [default: 1.1]
      --fullscreen
          Fullscreen VLC playback
      --vlc-timeout <VLC_TIMEOUT>
          Timeout in seconds for VLC startup [default: 60]
      --vlc-poll-interval <VLC_POLL_INTERVAL>
          Polling interval in milliseconds for VLC status checks [default: 100]
      --selection-p <SELECTION_P>
          Iterate top-scored entries and select the first where rand() <= p
  -h, --help
          Print help
```

### score

```text
$ classi-cine score --help
Score files using trained classifiers without interactive classification

Usage: classi-cine score [OPTIONS] <PLAYLIST> [DIRS]...

Arguments:
  <PLAYLIST>  M3U playlist file
  [DIRS]...   Directories to scan for video files

Options:
      --exclude <GLOB>
          Glob patterns of files and directories to exclude from the walk, matched against the normalized absolute path. A directory that matches is pruned entirely (its subtree is never descended into). Uses gitignore-flavored rules: a pattern with **no slash** is matched against the file/directory *name* (so `*.tmp` excludes any `.tmp` file at any depth and `sample` prunes any directory *or* file named `sample` anywhere); a pattern **with a slash** is matched against the full absolute path (so `**/trailers/**` excludes files under any `trailers` dir and `/abs/dir/**` anchors to a specific path). `globset` with `literal_separator(true)` applies: `*` matches a single path component and `**` spans any number of them. May be given multiple times; all patterns are OR-ed together
      --video-exts <VIDEO_EXTS>
          Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>
          Maximum contiguous window size for ngram features. Set to 0 to disable windows and rely solely on --combinations [default: 5]
      --combinations <COMBINATIONS>
          Generate orderless combinations of up to k tokens (default pairs) as ngram features. Independent of --windows, so --windows=0 leaves only combinations; set --combinations=0 to disable them entirely [default: 2]
      --file-size-bias <FILE_SIZE_BIAS>
          Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET>
          Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>
          Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET>
          Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>
          Bias scoring based on file age (log base, > 1.0). Negative reverses bias (older files get higher score)
      --file-age-offset <FILE_AGE_OFFSET>
          Offset to add to file age in seconds before log scaling [default: 86400]
      --cache-ttl-days <CACHE_TTL_DAYS>
          Cache TTL in days for the ffprobe feature cache. Entries whose key is not seen among the collected files for this long are expired. 0 disables expiry entirely (useful for cold, stable libraries). To force expire everything, delete the cache directory [default: 30]
      --features-combinations <FEATURES_COMBINATIONS>
          Orderless cross-feature combination order. Feature tokens (categorical singletons + per-continuous-feature neighbor singletons) are fed to `Ngrams::combinations` at this order, producing cross-feature ngrams like `{video_codec:h264, duration:21}`. `0` disables feature ngrams entirely (the feature `combinations` call is skipped). Independent of `--combinations`. See `docs/media-features-classifier.md` [default: 2]
      --features-smoothing <FEATURES_SMOOTHING>
          Neighbor smoothing half-width for continuous buckets. A value in bucket `i` also emits its immediate neighbors `[i-w, i+w]` (clamped to `>= 0`), so adjacent buckets share signal through overlapping singletons. `0` disables smoothing (plain 1-bucket singletons) [default: 1]
      --features-bucket-base <FEATURES_BUCKET_BASE>
          Geometric bucket base for `duration` / `filesize` / `bitrate` (> 1.0). `bucket(v) = floor(log_base(max(v, 1.0)))`. Power-of-2 bases are too coarse for media; 1.5 yields ~22 duration buckets across 1s–4h [default: 1.5]
      --features-fps-base <FEATURES_FPS_BASE>
          Geometric bucket base for `fps` (> 1.0), separate from `--features-bucket-base` because the fps range is narrow (~10–120) and clustered at standard rates. 1.1 keeps NTSC/PAL partners (23.976/24, 29.97/30, 59.94/60) in single buckets while separating adjacent groups [default: 1.1]
      --include-classified
          Include already classified files in the score listing
      --no-header
          Skip header output for machine-readable format
      --include-size
          Include file size in bytes in output
      --json
          Output results in JSON format
      --reverse
          Reverse output order (lowest scores first)
      --by-dir
          Group results by directory and aggregate scores
      --absolute
          Display absolute paths instead of relative to current directory
  -h, --help
          Print help
```

### list-positive / list-negative

```text
$ classi-cine list-positive --help
List positively classified files

Usage: classi-cine list-positive [OPTIONS] <PLAYLIST>

Arguments:
  <PLAYLIST>  M3U playlist file

Options:
      --absolute  Display absolute paths instead of relative to current directory
      --exists    Only print entries whose file still exists on disk
  -h, --help      Print help
```

`list-negative` takes the same arguments and prints negatively classified files.

### move

```text
$ classi-cine move --help
Move playlist to a new location and rebase paths

Usage: classi-cine move <ORIGINAL> <NEW>

Arguments:
  <ORIGINAL>  Original M3U playlist file
  <NEW>       New M3U playlist file location

Options:
  -h, --help  Print help
```

### reconcile

```text
$ classi-cine reconcile --help
Reconcile playlist with disk: drop bare lines for deleted files and re-add them for files that reappeared

Usage: classi-cine reconcile <PLAYLIST>

Arguments:
  <PLAYLIST>  M3U playlist file

Options:
  -h, --help  Print help
```

## How it ranks

Several classifiers score each candidate; their normalized scores combine into the final ranking:

- **Naive Bayes over path ngrams** — byte-pair encoding tokenizes file and folder names; orderless combinations learn which tokens tend to appear together in kept vs. skipped files. Language- and encoding-agnostic.
- **Naive Bayes over media features** — ffprobe-derived codec, resolution, aspect ratio, duration, bitrate, and fps are bucketed and smoothed into tokens that feed the Naive Bayes instance. See `docs/media-features-classifier.md`.
- **File-size, directory-size, and file-age classifiers** — logarithmic biases you can tune or reverse per dimension.

Under the hood:

- **ffprobe cache** — a persistent, sharded, TTL-based cache under your XDG cache dir amortizes probing across runs. A corrupt shard or a failed probe is skipped, never fatal; delete the cache directory to reset. See `docs/ffprobe-cache.md`.
- **Adaptive tokenization** — BPE learns frequent character sequences from *your* library.
- **Incremental learning** — each keep/skip updates the model immediately.
- **Background VLC control** — multi-threaded, via VLC's HTTP interface, with automatic process cleanup on exit.

## Contributing

Contributions welcome — enhancements, bug fixes, docs, and more.

## License

MIT. See [LICENSE](LICENSE).

______________________________________________________________________

Made with ❤️ and Rust.
