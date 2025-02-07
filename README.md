![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Classi-Cine** is a Rust-based tool that combines multiple classifiers for
intelligent video organization. It uses file path characteristics along with
user feedback through VLC's playback controls to build smart playlists.

The system employs multiple classification approaches:
- Naive Bayes classification of filename features (words, tokens, ngrams)
- File size analysis (Optional)
- Directory size/density patterns (Optional)

This multi-classifier approach allows the system to learn from both content patterns
and file organization structures. For example, you could use this as a video 
recommendation engine for local video files, with the system learning from both 
filename patterns and your existing file organization.

## Key Features

- **Multi-Classifier System:** Combines file size, directory patterns, and filename token analysis
  for more accurate recommendations
- **Interactive Training:** Pause (Shortcut: space) to mark as positive or
  stop (Shortcut: s) to mark as negative
- **Smart Re-ranking:** Multiple classifiers collaborate to re-rank videos based on
  combined scores and user feedback
- **M3U Playlist Integration:** Classifications are stored in standard M3U format
  for compatibility with media players
- **Efficient Processing:** Uses Bloom filters and parallel processing for
  handling large video collections

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

## How it works

1. **File Discovery:** Recursively locates video files in specified directories,
   filtering out previously classified entries
2. **Text Processing Pipeline:**
   - Normalizes filenames for consistent processing
   - Builds efficient tokenizer from all paths
   - Generates and identifies frequent n-grams
   - Uses Bloom filters for fast feature lookup
3. **Multi-Classifier Processing:**
   - Initializes file size, directory size, and Naive Bayes classifiers
   - Processes entries to establish scoring bounds
   - Normalizes and combines scores from all classifiers
4. **Interactive Classification:**
   - Presents highest scoring candidates first
   - Launches VLC with HTTP interface
   - Captures user feedback through playback controls
   - Updates all classifiers and playlist data
   - Continuously re-ranks remaining entries

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation
improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
