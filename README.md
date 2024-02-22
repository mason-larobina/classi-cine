![classi-cine](classi-cine.png)

# Classi-Cine

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

[![Rust](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml/badge.svg)](https://github.com/mason-larobina/classi-cine/actions/workflows/rust.yml)

**Classi-Cine** is a Rust-based tool that utilizes a Naive Bayes classifier for
filename-based video tagging. It offers a user-directed classification approach
by interacting with VLC's playback states via its http interface.

The first iteration of this tool is being used to help find and tag videos for
deletion based on learned features (words, tokens, ngrams) in the video
filepaths. The tag names "keep" and "delete" are arbitrary and can be
overridden. For example, you could use this as a video recommendation engine for
local video files. Stopping playback will train and recommend other similar
video files.

## Key Features

- **Interactive Training:** Pause (Shortcut: space) to tag a video as "keep" or
  stop (Shortcut: s) to tag it as "delete".
- **Dynamic Re-ranking:** The classifier updates and re-ranks videos based on
  user input.
- **Customizable Tags:** The "keep" and "delete" tags can be customized, making
  this tool versatile for different applications.

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
Usage: classi-cine [OPTIONS] <PATHS>...

Arguments:
  <PATHS>...

Options:
      --tokenize <TOKENIZE>
          The tokenizer to use [default: chars] [possible values: words, chars]
      --windows <WINDOWS>
          Create ngrams (windows of tokens) from 1 to N [default: 20]
      --delete <DELETE>
          The text file containing the files to delete [default: delete.txt]
      --keep <KEEP>
          The text file containing the files to keep [default: keep.txt]
      --log-level <LOG_LEVEL>
          [default: info]
  -f, --fullscreen
          Fullscreen VLC playback
      --file-size-log-base <FILE_SIZE_LOG_BASE>
          The log base for the file size which is mixed into the classifier score to preference larger files over smaller files. Recommended values are close to 1.0, for example 1.1, 1.01, 1.001, and so on
      --vlc-port <VLC_PORT>
          [default: 9010]
      --video-exts <VIDEO_EXTS>
          [default: avi,flv,mov,f4v,flv,m2ts,m4v,mkv,mpg,webm,wmv,mp4]
  -h, --help
          Print help
```

## How it works

1. **Discover Video Files:** Locates all video files within the given
   directories.
1. **Tokenization:** Words in the video file paths are tokenized and
   post-processed to handle unique and common tokens.
1. **Initialize Classifier:** Loads previous tag states from file lists into the
   classifier.
1. **Re-ranking:** Determines the next untagged video files to process ranked by
   most likely to be tagged based on the features in the video filepath.
1. **Interact with VLC:** Launches VLC with the http interface.
1. **User Feedback Loop:** The classifier and video ranking adapt based on
   whether playback is paused or stopped to train the classifier, re-rank and
   open the next likely video for tagging.

## Contributing

We're open to contributions! Enhancements, bug fixes, documentation
improvements, and more are all welcome.

## License

This project is licensed under the MIT License. See LICENSE for details.

## Special Thanks

Made with ❤️ and Rust.
