![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Stop scrolling through thousands of videos.** Classi-Cine is like autocomplete for video selection - it predicts what you want to watch next based on your feedback, helping you rapidly build the perfect playlist for your current mood.

## The Problem

You have a large video collection but finding what matches your current mood is tedious:

- Manually browsing through hundreds or thousands of files
- Remembering which episodes or clips you enjoyed
- No quick way to find "videos like this one"
- Building playlists by hand takes forever

## The Solution

Classi-Cine plays 20-questions with your video collection to rapidly zero in on what you want:

1. **Point it at your video directories** - Let it discover your collection
1. **It suggests videos, you accept/reject** - Like/dislike using VLC controls (stop = accept, pause = reject)
1. **Watch recommendations improve in real-time** - Each decision teaches the AI what you're looking for
1. **Build your perfect playlist faster** - Find content that matches your current mood in minutes, not hours

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
# Stop (s) to accept and add to playlist, pause (space) to reject
# Watch suggestions get smarter with each decision!
```

## How It Works

Like autocomplete for text, Classi-Cine learns patterns from your video collection and predicts what you want next:

1. **Analyzes your entire collection** - Understands file names, folder structure, sizes, and ages
1. **Uses machine learning** - Multiple AI classifiers work together to score every video
1. **Learns from your feedback** - Each accept/reject teaches it more about your preferences
1. **Predicts better matches** - Rankings improve in real-time as you build your playlist

**Simple VLC Integration:**

- Stop video (s key) = "Yes, add this to my playlist"
- Pause video (space) = "No, this doesn't fit"
- Standard M3U playlists work in any media player

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
  list-positive  List classified files
  list-negative  
  move           Move playlist to a new location and rebase paths
  help           Print this message or the help of the given subcommand(s)

Options:
      --log-level <LOG_LEVEL>  [default: info]
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
      --video-exts <VIDEO_EXTS>         Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>               [default: 5]
      --file-size-bias <FILE_SIZE_BIAS> Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET> Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>   Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET> Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>   Bias scoring based on file age (log base, > 1.0). Negative reverses bias
      --file-age-offset <FILE_AGE_OFFSET> Offset to add to file age in seconds before log scaling [default: 86400]
      --fullscreen                      Fullscreen VLC playback
      --vlc-timeout <VLC_TIMEOUT>       Timeout in seconds for VLC startup [default: 60]
      --vlc-poll-interval <VLC_POLL_INTERVAL> Polling interval in milliseconds for VLC status checks [default: 100]
      --batch <BATCH>                   Number of entries to classify in each batch iteration [default: 1]
      --random-top-n <RANDOM_TOP_N>     Select next entry randomly from top-n scored entries
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
      --video-exts <VIDEO_EXTS>         Video file extensions to scan for [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
      --windows <WINDOWS>               [default: 5]
      --file-size-bias <FILE_SIZE_BIAS> Bias scoring based on file sizes (log base, > 1.0). Negative reverses bias
      --file-size-offset <FILE_SIZE_OFFSET> Offset to add to file size before log scaling [default: 1048576]
      --dir-size-bias <DIR_SIZE_BIAS>   Bias scoring based on directory sizes (log base, > 1.0). Negative reverses bias
      --dir-size-offset <DIR_SIZE_OFFSET> Offset to add to directory size before log scaling [default: 0]
      --file-age-bias <FILE_AGE_BIAS>   Bias scoring based on file age (log base, > 1.0). Negative reverses bias
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

## Technical Details

For developers and ML enthusiasts, Classi-Cine uses several sophisticated techniques:

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
