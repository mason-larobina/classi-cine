# Classi-cine Design Document

## Overview

Classi-cine is an intelligent video recommendation tool that combines multiple
machine learning approaches with interactive user feedback to efficiently build
playlists of related videos.

It addresses common challenges in managing video libraries:

- Time-consuming manual organization of videos
- Inconsistent naming conventions and file locations
- Need to preview content before classification
- Desire to learn from user preferences over time

The system uses VLC media player for previews and classification input, making
it immediately familiar to users while providing a robust foundation for video
playback across formats and platforms.

## Core Components

### 1. Multi-Classifier System

The Naive Bayes Classifier extracts meaning from filenames themselves. While
individual words or tokens might be ambiguous, analyzing patterns of character
sequences (n-grams) helps identify naming conventions and content indicators.
This is particularly valuable when dealing with inconsistent naming schemes or
multiple languages.

The File Size Classifier can be configured to favour either larger or smaller
files. This is particularly useful when file size correlates with content
quality or length - for example, preferring high-bitrate files for archival
purposes, or smaller files for casual viewing.

The Directory Size Classifier can be configured to favour files in densely
populated directories or sparsely populated directories. In some cases densely
populated directories could indicate duplicate or redundant video content in
which the user may wish to select the best-of and delete the worst-of.

### 2. Text Processing Pipeline

// TODO: improve this section

The text processing pipeline attempts to make sense of filenames in a context
free way. Raw filenames come in countless formats and styles, often mixing
different conventions, components, languages, and special characters. The
pipeline begins with normalization, converting these varied inputs into a
standardized form where patterns can be more reliably detected.

Rather than relying on traditional word splitting, which often fails with
filenames, the system uses pair encoding tokenization. This approach learns
from the data itself, identifying common character pairs that tend to mark word
boundaries. This makes it remarkably effective at handling arbitrary naming
conventions and multiple languages without requiring predefined rules.

The final stage generates n-grams - overlapping sequences of tokens that
capture more context than individual words alone. This helps identify
meaningful phrases and patterns, even when the original filename uses
unconventional formatting or lacks clear word boundaries. The result is a
robust feature extraction system that works across a wide range of naming
styles.

### 3. VLC Integration

The system integrates with VLC media player to create a seamless classification
workflow. By leveraging VLC's built-in HTTP interface, it achieves programmatic
control without requiring any modifications to VLC itself.

Classification controls are designed to feel natural during video preview -
stopping marks content (default: s) as positive, while pausing (default: space)
marks it as negative. This lets users make quick decisions while watching,
maintaining an efficient workflow even when processing large collections.

### 4. Playlist Management

The system stores classifications in M3U playlists, a universal format that
works with virtually any media player. This simple text-based format ensures
that classification results remain useful even outside the tool itself - users
can immediately start using their organized playlists in their preferred media
player.

The system stores positive classifications as standard entries in the M3U
playlist, making them immediately usable in any media player. Negative
classifications are stored with a special prefix in the playlist metadata
section, which standard players ignore. This dual storage approach maintains
compatibility with existing media players while preserving the full
classification history for training.

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
decisions to improve future recommendations. Each classification not only
organizes the current file but also helps train the system to better understand
user preferences. This creates a virtuous cycle where the system becomes
increasingly attuned to the user's organization style over time.

## Performance Considerations

Processing large video collections requires careful attention to performance.
The system manages thread priorities to ensure smooth video playback during
classification - background tasks like feature extraction run at lower
priority, preventing them from interfering with the user interface and video
preview.

Modern CPUs offer significant parallel processing capability, which the system
leverages through careful workload distribution. Tasks like filename analysis
and feature extraction run in parallel where possible, significantly reducing
processing time for large collections.

Bloom filters provide an elegant solution for fast feature lookups without
excessive memory use. This probabilistic data structure lets the system quickly
check if a file might have certain features, avoiding unnecessary detailed
analysis of files that couldn't possibly match the current classification
patterns.

When processing data in parallel, contention for shared resources can become a
bottleneck. The system uses sharded data structures to distribute work across
multiple independent containers, reducing lock contention and allowing better
scaling across CPU cores. This is particularly important when processing large
directories with many files.

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
