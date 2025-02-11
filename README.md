![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Classi-Cine** is a Rust-based tool that combines multiple classifiers for
intelligent video organization. It uses file path characteristics along with
user feedback through VLC's playback controls to build smart playlists.

## Overview

Classi-cine helps organize video libraries by learning from both content
patterns and your feedback. It combines multiple classification approaches:

- **Smart Multi-Classifier System:**

  - Byte pair encoding tokenization learns frequent character sequences
  - Naive Bayes classification of filename tokens and token sequences
  - Language and character set agnostic
  - Optional file size classification (prefer larger/smaller files)
  - Optional directory size classifier (prefer files in large/smaller
    directories)

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

- Uses probabilistic filters and parallel processing for fast and efficient
  tokenization
- Adaptive tokenization learns from your library's naming patterns
- Sharded data structures for multi-core processing

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
Usage: classi-cine [OPTIONS] <COMMAND>

Commands:
  build          Build playlist through interactive classification
  list-positive  List positively classified files
  list-negative  List negatively classified files
  help           Print this message or the help of the given subcommand(s)

Options:
      --log-level <LOG_LEVEL>  [default: info]
  -h, --help                   Print help
```

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation
improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
