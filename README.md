![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Classi-Cine** is a Rust-based tool that combines multiple classifiers for
intelligent video organization. It uses file path characteristics along with
user feedback through VLC's playback controls to build smart playlists.

## Overview

Classi-cine helps solve common challenges in managing video libraries:
- Organizing videos with inconsistent file names and locations
- Quick preview and classification of content
- Learning from user preferences over time

The system employs multiple classification approaches:
- Naive Bayes classification of filename features (words, tokens, ngrams)
- File size analysis (Optional)
- Directory size/density patterns (Optional)

This multi-classifier approach allows the system to learn from both content patterns
and file organization structures. For example, you could use this as a video 
recommendation engine for local video files, with the system learning from both 
filename patterns and your existing file organization.

## Key Features

- **Smart Multi-Classifier System:**
  - Analyzes filename patterns, common words, and character sequences
  - Optional file size classification (prefer larger/smaller files)
  - Optional directory density analysis
  - Combines multiple signals for better recommendations

- **Text Processing:**
  - Byte pair encoding tokenization learns frequent character sequences
  - Naive Bayes classification of filename tokens
  - Language and character set agnostic

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

- Uses Bloom filters and parallel processing for efficient tokenization
- Adaptive tokenization learns from your library's naming patterns
- Sharded data structures for multi-core processing
- Probabilistic filtering for fast token matching

## Installation

VLC is required for video playback.

Ensure you have Rust and Cargo installed. If not, you can install them using
rustup.

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
Usage: classi-cine [OPTIONS] <PLAYLIST> [DIRS]...

Arguments:
  <PLAYLIST>  M3U playlist file for storing classifications
  [DIRS]...   Directories to scan for video files (defaults to current directory) [default: .]

Options:
      --log-level <LOG_LEVEL>    [default: info]
  -f, --fullscreen               Fullscreen VLC playback
      --port <PORT>              [default: 9111]
      --vlc-port <VLC_PORT>      [default: 9010]
      --video-exts <VIDEO_EXTS>  [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
  -h, --help                     Print help
```

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation
improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
