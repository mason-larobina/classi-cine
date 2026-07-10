![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Autocomplete, but for your video library.** Classi-Cine watches which videos you keep and which you skip, then predicts what you'll want next - so you build the perfect playlist in a few clicks instead of scrolling for an hour.

## The Problem

A big video collection is only useful if you can find the right thing in it - and usually you can't:

- Scrolling through hundreds or thousands of files to find one
- Trying to remember which episodes or clips were actually good
- No quick way to say "more like this one"
- Hand-building playlists that take longer than watching them

## The Solution

Classi-Cine plays a fast round of accept/reject to home in on what you want:

1. **Point it at your video directories** - it discovers your whole collection
1. **It suggests, you decide** - accept or reject each pick with VLC controls (stop = keep, pause = skip)
1. **Suggestions sharpen as you go** - every decision teaches it your taste
1. **Walk away with a finished playlist** - built in minutes, not an afternoon

Perfect for:

- **Large media collections** (TV series, movies, documentaries, clips)
- **Content creators** curating references or footage
- **Anyone with 100+ videos** who wants smart discovery
- **Building themed playlists** (action movies, comedy episodes, tutorial clips, etc.)

## Quick Start

```bash
# Install
cargo install classi-cine

# Start building a playlist - the AI will suggest additions
classi-cine build my-playlist.m3u ~/Videos ~/Movies

# The TUI shows AI predictions ranked by relevance
# Press Enter to preview in VLC
# Stop (s) to keep and add to playlist, pause (space) to skip
# Watch suggestions get smarter with each decision!
```

## How It Works

Just like text autocomplete learns from what you type, Classi-Cine learns from what you keep:

1. **Reads your whole collection** - file names, folder structure, sizes, and ages
1. **Scores every video** - several lightweight classifiers rank each candidate
1. **Learns from each call** - every keep/skip updates its model of your taste
1. **Re-ranks in real time** - the next suggestion is always its best guess

**Simple VLC integration:**

- Stop video (s key) = "Yes, keep this in my playlist"
- Pause video (space) = "No, skip it"
- Standard M3U playlists work in any media player

## Recipe: Cull a Folder of Clips

A favorite use is the inverse of building a "keep" list: quickly triaging a messy folder of clips and deleting the ones you don't want.

The trick is to flip the meaning of the labels. Classi-Cine marks stopped videos as positive, so let **positive mean "delete"** - press stop (s) on clips you want gone and pause (space) on the keepers. As you classify, the playlist learns the patterns of the unwanted clips and surfaces more of them, so culling gets faster the longer you go.

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

VLC is required for video playback.

Ensure you have Rust and Cargo installed. If not, you can install them using rustup.

### From Cargo

```bash
# Build from the cargo.io crate registry.
$ cargo install classi-cine
```

### From Source

```bash
# Clone this repository
$ git clone https://github.com/mason-larobina/classi-cine.git

# Go into the repository
$ cd classi-cine

# Build and install it locally
$ cargo install --path=.
```

## Usage

```bash
Usage: classi-cine [OPTIONS] <COMMAND>

Commands:
  build          Build playlist through interactive classification
  score          Score files using trained classifiers without interactive classification
  list-positive  List positively classified files
  list-negative  List negatively classified files
  move           Move playlist to a new location and rebase paths
  reconcile      Reconcile playlist with disk (drop deleted, re-add reappeared files)
  help           Print this message or the help of the given subcommand(s)

Options:
      --log-level <LOG_LEVEL>  [default: info]
      --log-file <LOG_FILE>    Write log output to this file. When set, logs always go to the file, even while the interactive TUI is running (which suppresses stderr logs)
  -h, --help                   Print help
```

### Build Command

Build a playlist through interactive VLC classification:

```bash
classi-cine build [OPTIONS] <PLAYLIST> [DIRS]...

Arguments:
  <PLAYLIST>  M3U playlist file
  [DIRS]...   Directories to scan for video files

Options:
      --exclude <GLOB>                 Glob patterns of files/dirs to exclude (gitignore-flavored: no-slash matches basename anywhere, with-slash matches absolute path). Repeatable. A matching directory is pruned entirely
      --video-exts <VIDEO_EXTS>         Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>               [default: 5]
      --file-size-bias <FILE_SIZE_BIAS> Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET> Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>   Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET> Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>   Bias scoring based on file age (log base, > 1.0). Negative reverses bias (older files get higher score)
      --file-age-offset <FILE_AGE_OFFSET> Offset to add to file age in seconds before log scaling [default: 86400]
      --fullscreen                      Fullscreen VLC playback
      --vlc-timeout <VLC_TIMEOUT>       Timeout in seconds for VLC startup [default: 60]
      --vlc-poll-interval <VLC_POLL_INTERVAL> Polling interval in milliseconds for VLC status checks [default: 100]
      --selection-p <SELECTION_P>       Iterate top-scored entries and select the first where rand() <= p
  -h, --help                            Print help
```

### Score Command

Score files using trained classifiers without interactive classification:

```bash
classi-cine score [OPTIONS] <PLAYLIST> [DIRS]...

Arguments:
  <PLAYLIST>  M3U playlist file
  [DIRS]...   Directories to scan for video files

Options:
      --exclude <GLOB>                 Glob patterns of files/dirs to exclude (gitignore-flavored: no-slash matches basename anywhere, with-slash matches absolute path). Repeatable. A matching directory is pruned entirely
      --video-exts <VIDEO_EXTS>         Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>               [default: 5]
      --file-size-bias <FILE_SIZE_BIAS> Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET> Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>   Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET> Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>   Bias scoring based on file age (log base, > 1.0). Negative reverses bias (older files get higher score)
      --file-age-offset <FILE_AGE_OFFSET> Offset to add to file age in seconds before log scaling [default: 86400]
      --include-classified              Include already classified files in the score listing
      --no-header                       Skip header output for machine-readable format
      --include-size                    Include file size in bytes in output
      --json                            Output results in JSON format
      --reverse                         Reverse output order (lowest scores first)
      --by-dir                          Group results by directory and aggregate scores
      --absolute                        Display absolute paths instead of relative to current directory
  -h, --help                            Print help
```

### List Commands

List positively or negatively classified files:

```bash
classi-cine list-positive [OPTIONS] <PLAYLIST>
classi-cine list-negative [OPTIONS] <PLAYLIST>

Arguments:
  <PLAYLIST>  M3U playlist file

Options:
      --absolute  Display absolute paths instead of relative to current directory
      --exists    Only print entries whose file still exists on disk
  -h, --help      Print help
```

### Move Command

Move playlist to a new location and rebase paths:

```bash
classi-cine move <ORIGINAL> <NEW>

Arguments:
  <ORIGINAL>  Original M3U playlist file
  <NEW>       New M3U playlist file location

Options:
  -h, --help  Print help
```

### Reconcile Command

Rewrite the playlist file so its on-disk form matches the current state of disk: bare filename lines are dropped for positive entries whose files have been deleted, and re-added for files that reappeared. The `#{...}` classification metadata is always preserved so training data survives.

```bash
classi-cine reconcile <PLAYLIST>

Arguments:
  <PLAYLIST>  M3U playlist file

Options:
  -h, --help  Print help
```

## Technical Details

Under the hood, Classi-Cine combines several techniques to turn your keep/skip calls into accurate rankings:

**Multi-Classifier Architecture:**

- **Naive Bayes Classifier**: Uses byte-pair encoding tokenization to learn patterns from file paths and folder names
- **File Size Classifier**: Logarithmic scoring based on file sizes (configurable bias for larger/smaller files)
- **Directory Size Classifier**: Scoring based on number of files in directories
- **File Age Classifier**: Scoring based on file creation time
- All classifiers use normalized scores that are combined for final ranking

**Advanced Tokenization:**

- Byte pair encoding learns frequent character sequences from your specific collection
- Language and character set agnostic - works with any naming convention
- Adaptive tokenization learns from your library's patterns
- N-gram analysis for sequence pattern recognition

**Performance Optimizations:**

- Probabilistic filters and parallel processing for efficient tokenization
- Sharded data structures for multi-core processing
- Background VLC control with multi-threaded communication
- Incremental M3U saves for long classification sessions

**VLC Integration:**

- Uses VLC's HTTP interface for status monitoring
- Multi-threaded VLC control with background processing
- Automatic process lifecycle management
- Real-time classification feedback loop

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
