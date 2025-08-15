![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Classi-Cine** is a Rust-based tool that combines multiple classifiers for intelligent video organization. It uses file path characteristics along with user feedback through VLC's playback controls to build smart playlists.

## Overview

Classi-cine helps organize video libraries by learning from both content patterns and your feedback. It combines multiple classification approaches:

- **Smart Multi-Classifier System:**

  - Byte pair encoding tokenization learns frequent character sequences
  - Naive Bayes classification of filename tokens and token sequences
  - Language and character set agnostic
  - Optional file size classification (prefer larger/smaller files)
  - Optional directory size classifier (prefer files in large/smaller directories)

- **Seamless VLC Integration:**

  - Uses familiar VLC controls for feedback
  - Stop video (s key) = positive classification
  - Pause video (space) = negative classification
  - Immediate playlist updates

- **Universal Playlist Format:**

  - Stores results in standard M3U format
  - Compatible with most media players
  - Preserves classification history
  - Incremental saves for long sessions

## Technical Details

- Uses probabilistic filters and parallel processing for fast and efficient tokenization
- Adaptive tokenization learns from your library's naming patterns
- Sharded data structures for multi-core processing

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

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
