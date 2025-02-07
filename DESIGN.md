# Classi-cine Design Document

## Overview

Classi-cine is an intelligent video recommendation tool that combines a mix of
bayesian, static and dynamic classifiers with user feedback to efficiently
build a playlist of related videos.

It addresses common challenges in managing video libraries:

- Time-consuming manual organization of videos from disorganized file names and locations
- Need to preview content before classification
- Learn from user preferences over time

The system uses VLC media player for previews and classification input, making
it immediately familiar to users while providing a robust foundation for video
playback across formats and platforms.

## Core Components

### 1. Multi-Classifier System

The Naive Bayes Classifier extracts meaning from tokenized filenames and user
feedback. While individual words or tokens might be ambiguous, analyzing
patterns of character sequences (n-grams) helps identify common strings, names,
studios and content indicators. This is particularly valuable when dealing with
inconsistent naming schemes or multiple languages.

The File Size Classifier can be configured to favour either larger or smaller
files. This is particularly useful when file size correlates with content
quality or length - for example, preferring high-bitrate files for archival
purposes, or smaller files for casual viewing.

The Directory Size Classifier can be configured to favour files in densely
populated directories or sparsely populated directories. In some cases densely
populated directories could indicate duplicate or redundant video content in
which the user may wish to select the best-of and delete the worst-of.

### 2. Text Processing Pipeline

The text processing pipeline transforms raw filepaths into meaningful features
through three stages:

1. Normalization
   - Converts text to lowercase for case-insensitive matching
   - Replaces special characters with spaces while preserving path separators
   - Collapses multiple spaces/separators into singles
   - Removes apostrophes and trailing spaces
   
   Examples:
   ```
   "The.Quick-Brown.Fox.mp4" -> "the quick brown fox mp4"
   "Action/SciFi!!!Movie's.mkv" -> "action/scifi movies mkv"
   "Multiple     Spaces.avi" -> "multiple spaces avi"
   ```

2. Tokenization
   - Initially splits text into individual characters
   - Learns common character pairs from the corpus
   - Iteratively merges frequent pairs into larger tokens
   - Preserves special tokens like path separators
   
   Examples:
   Initial: "s|c|i|f|i| |m|o|v|i|e"
   After merges: "sci|fi| |movie"
   
   The pair tokenizer adapts to naming conventions in the corpus:
   ```
   Training corpus:
   "comedy movie.mp4"
   "scifi documentary.avi"
   "scifi movie.mp4"
   "scifi series.mkv" 
   "western movie.mp4"
   
   Learned merges:
   's'+'c' -> 'sc'
   'sc'+'i' -> 'sci'
   'f'+'i' -> 'fi'
   'm'+'o' -> 'mo'
   'mo'+'v' -> 'mov'
   'mov'+'i' -> 'movi'
   'movi'+'e' -> 'movie'
   ```

3. N-grams
   - Generates overlapping windows of tokens
   - Filters unique n-grams for efficient classification
   - Uses bloom filters for efficient probabilistic matching and replacement of
     frequent token pairs
   
   Example with window size [1..4]:
   Tokens: ["sci", "fi", "movie"]
   N-grams: ["sci", "sci-fi", "sci-fi-movie", "fi-movie", "movie"]
   
This pipeline effectively handles:
- Inconsistent naming conventions
- Multiple languages and character sets
- Common abbreviations and patterns
- Efficient matching at scale

### 3. VLC Integration

This tool uses VLC and the VLC's built-in HTTP interface to inspect and control
the VLC playlist and playback state for user classification feedback.

Stopping a video (default key: s) is a positive classification (more of this)
and pausing a video (default key: space) is a negative classification (less
like this). This lets users make quick decisions while watching, maintaining an
efficient workflow even when processing large collections.

### 4. Playlist Management

The system stores classifications in M3U playlists, a universal format that
works with virtually any media player. This simple text-based format ensures
that classification results remain useful even outside the tool itself - users
can immediately start using their organized playlists in their preferred media
player.

The system stores positive classifications as standard entries in the M3U
playlist. Negative classifications are stored with a special metadata prefix in
the playlist, which standard players ignore. This dual storage approach
maintains compatibility with existing media players while preserving the full
classification history for training and future classification sessions.

Classifications are saved incrementally, with each decision being immediately
appended to the appropriate playlist. This ensures that progress is preserved
even during long classification sessions, and users can safely pause and resume
their organization efforts at any time.

## Data Flow

The system processes video files through several carefully orchestrated stages.
It begins with comprehensive file collection, recursively scanning directories
to build a complete view of the video library. This initial scan filters out
previously classified content and focuses only on supported video formats,
ensuring efficient processing of relevant files.

The text processing stage transforms raw filenames into meaningful features.
This is crucial because filenames often contain valuable information about
content, but in widely varying formats. The multi-stage pipeline normalizes
these names and extracts patterns that help identify related content, building
up a rich set of features for classification.

Classification combines signals from multiple sources to prioritize which files
to review first. By analyzing file characteristics, directory structures, and
learned patterns, the system can present the most relevant files early in the
review process. This intelligent ordering helps users find related content more
quickly and makes better use of classification time.

The interactive loop ties everything together, continuously learning from user
decisions to improve future recommendations. Each classification builds the
targeted playlist and also helps train the system to better understand user
preferences for that playlist. This creates a virtuous cycle where the system
becomes increasingly attuned to the user's playlist preferences over time.

## Performance Considerations

Modern CPUs offer significant parallel processing capability, which the system
leverages through careful workload distribution. Tasks like filename analysis
and feature extraction run in parallel where possible, significantly reducing
processing time for large collections.

Bloom filters are used for fast probabilistic filtering of token lists to
update with token merges avoiding unnecessary processing of unrelated tokens.

In some cases sharded data structures are used to distribute work across CPU
cores.

## Future Improvements

Planned enhancements to extend functionality:

- Additional Classifier Types
  - Video metadata (length, format, image dimensions)

- Web interface
  - Remote control 
  - Classification visualization & inspection

- Analysis Tools
  - Pattern discovery
  - Group classification suggestions

- Classification Export
  - JSON output format
  - Database integration
