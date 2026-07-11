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

```bash
classi-cine [OPTIONS] <COMMAND>
```

| Command | Purpose |
| ---------------- | ------------------------------------------------------------- |
| `build` | Build a playlist through interactive classification |
| `score` | Rank files using trained classifiers (no interaction) |
| `list-positive` | List positively classified files |
| `list-negative` | List negatively classified files |
| `move` | Move a playlist to a new location and rebase its paths |
| `reconcile` | Drop deleted files and re-add reappeared ones |
| `help` | Print help for a command |

Global options: `--log-level` (default `info`), `--log-file` (log to a file, always, even while the TUI runs), `-h`/`--help`. Run `classi-cine --help` for the full listing.

### build

```bash
classi-cine build [OPTIONS] <PLAYLIST> [DIRS]...
```

Discover videos, train on your keep/skip calls, and interactively build a playlist. Notable options:

- `--exclude <GLOB>` — gitignore-flavored exclude patterns, repeatable.
- `--video-exts` — extensions to scan (default: common video formats).
- `--windows`, `--combinations` — ngram window size and orderless-combination order for path tokens.
- `--file-size-bias`, `--dir-size-bias`, `--file-age-bias` — log-base biases (negative reverses); each has a matching `--*-offset`.
- `--cache-ttl-days` — ffprobe feature cache TTL (default 30; 0 = never expire).
- `--features-combinations`, `--features-smoothing`, `--features-bucket-base`, `--features-fps-base` — tune the media-features classifier (see `docs/media-features-classifier.md`).
- `--fullscreen`, `--selection-p`, `--vlc-timeout`, `--vlc-poll-interval`.

Run `classi-cine build --help` for the full list.

### score

```bash
classi-cine score [OPTIONS] <PLAYLIST> [DIRS]...
```

Train on the classifications already in a playlist, then rank discovered files by combined score — no VLC interaction. Shares `build`'s discovery, bias, cache, and feature options, plus output controls:

- `--include-classified` — include already-classified files in the listing.
- `--by-dir` — group results by directory and aggregate scores.
- `--json`, `--no-header`, `--include-size`, `--reverse`, `--absolute`.

### list-positive / list-negative

```bash
classi-cine list-positive [OPTIONS] <PLAYLIST>
classi-cine list-negative [OPTIONS] <PLAYLIST>
```

Options: `--absolute`, `--exists` (only files still on disk).

### move

```bash
classi-cine move <ORIGINAL> <NEW>
```

Write the playlist to a new location, rebasing its relative paths.

### reconcile

```bash
classi-cine reconcile <PLAYLIST>
```

Rewrite the playlist to match disk: drop bare lines for deleted files and re-add them for files that reappeared. `#{...}` classification metadata is always preserved, so training data survives.

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
