# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

- **Format code**: `./format.sh` (runs `cargo fmt` and `mdformat *.md`)
- **Build**: `cargo build` or `cargo +stable build`
- **Test**: `cargo test` or `cargo +stable test`
- **Run**: `cargo run -- <subcommand>` (see CLI usage below)
- **Install locally**: `cargo install --path=.`

## Project Architecture

Classi-Cine is a Rust CLI tool that uses machine learning to build smart video playlists by learning user preferences through VLC playback feedback.

### Core Components

**Main Application Flow (`app.rs`)**:
- `App` struct orchestrates the entire classification workflow
- Manages multiple classifiers, tokenization, and VLC integration
- Main phases: file collection → tokenization → ngram generation → training → classification loop

**Classification System (`classifier.rs`)**:
- **NaiveBayesClassifier**: Core ML classifier using ngram frequencies with Laplace smoothing
- **FileSizeClassifier**: Logarithmic scoring based on file sizes
- **DirSizeClassifier**: Scoring based on directory file counts
- **FileAgeClassifier**: Scoring based on file creation time
- All classifiers implement the `Classifier` trait with `calculate_score()` method

**VLC Integration (`vlc.rs`)**:
- `VlcController`: Multi-threaded VLC control with background processing
- Uses VLC's HTTP interface for status monitoring
- Classification mapping: stop = positive, pause = negative
- Automatic process lifecycle management with `Drop` traits

**Tokenization System**:
- `PairTokenizer` (`tokenize.rs`): Byte-pair encoding for path tokenization
- `Ngrams` (`ngrams.rs`): N-gram generation and frequency analysis
- `Tokens` (`tokens.rs`): Token representation and management

**Data Management**:
- `M3uPlaylist` (`playlist.rs`): M3U playlist format handling with relative path management
- `Walk` (`walk.rs`): Parallel file system traversal with video file filtering
- `normalize.rs`: Path normalization utilities

### Key Design Patterns

**Multi-threaded Architecture**: VLC control runs in background thread communicating via channels (`mpsc`)

**Classifier Composition**: Multiple classifiers with normalized scores combined for final ranking

**Incremental Learning**: Naive Bayes classifier updates with each user classification

**Resource Management**: Automatic cleanup with `Drop` implementations for VLC processes

## CLI Usage

```bash
classi-cine build <playlist.m3u> <directories...> [options]
classi-cine list-positive <playlist.m3u>
classi-cine list-negative <playlist.m3u>
classi-cine move <original.m3u> <new.m3u>
```

Key options for `build`:
- `--top-n`: Number of files to classify per iteration
- `--file-size-bias`: Logarithmic bias for file sizes
- `--dir-size-bias`: Logarithmic bias for directory sizes
- `--file-age-bias`: Logarithmic bias for file age
- `--dry-run`: Skip actual classification loop

## Testing and Quality

- Use `cargo test` for unit tests
- Use `./format.sh` before committing to ensure consistent formatting
- The publish script enforces clean git status before publishing
