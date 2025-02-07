# Classi-cine Design Document

## Overview

Classi-cine is an intelligent video recommendation tool that combines multiple
machine learning approaches with interactive user feedback to efficiently build
playlists of related videos.

It addresses common challenges in managing video libraries:

- Time-consuming manual organization
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

The File Size Classifier uses logarithmic scaling to handle the wide range of video file sizes. It can be configured to favor either larger or smaller files through its reverse flag. This is particularly useful when file size correlates with content quality - for example, preferring high-bitrate files for archival purposes, or smaller files for casual viewing. The logarithmic scaling ensures that differences between small files (e.g., 100MB vs 200MB) are as significant as proportional differences between large files (e.g., 1GB vs 2GB).

The Directory Size Classifier analyzes the number of files in each directory to identify content groupings. Like the file size classifier, it can be configured to favor either densely or sparsely populated directories. This helps capture different organization patterns - some users group related content together in large directories, while others prefer fine-grained categorization with fewer files per directory. The classifier's reverse flag lets it adapt to these different organizational styles.

### 2. Text Processing Pipeline

The text processing pipeline tackles one of the most challenging aspects of
video organization: making sense of filenames. Raw filenames come in countless
formats and styles, often mixing different conventions, languages, and special
characters. The pipeline begins with normalization, converting these varied
inputs into a standardized form where patterns can be more reliably detected.

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

The system stores positive classifications as standard entries in the M3U playlist, making them immediately usable in any media player. Negative classifications are stored with a special prefix in the playlist metadata section, which standard players ignore. This dual storage approach maintains compatibility with existing media players while preserving the full classification history for training. The metadata approach for negatives means users can share their playlists without exposing their negative classifications.

Classifications are saved incrementally, with each decision being immediately
appended to the appropriate playlist. This ensures that progress is preserved
even during long classification sessions, and users can safely pause and resume
their organization efforts at any time.

## Data Flow

The system processes files through several stages, each building on the previous:

1. File Collection
   - Why: Need complete view of video collection
   - How: Recursive directory scanning with filtering
   - Benefit: Handles any directory structure

2. Text Processing
   - Why: Raw filenames need preparation
   - How: Multi-stage pipeline with feedback
   - Benefit: Extracts maximum information from names

3. Classification
   - Why: Need to prioritize files for review
   - How: Multi-classifier scoring and ranking
   - Benefit: Presents most relevant files first

4. Interactive Loop
   - Why: User feedback improves accuracy
   - How: Continuous learning from classifications
   - Benefit: System improves over time

## Performance Considerations

Several techniques ensure efficient operation with large collections:

- Thread Priority Management
  - Why: Video playback must be smooth
  - How: Background processing uses lower priority
  - Benefit: Responsive UI during processing

- Parallel Processing
  - Why: Modern CPUs have multiple cores
  - How: Parallel algorithms where possible
  - Benefit: Faster processing of large collections

- Bloom Filters
  - Why: Need fast feature lookups
  - How: Probabilistic data structure
  - Benefit: Memory efficient pattern matching

- Sharded Data Structures
  - Why: Reduce contention in parallel code
  - How: Split data across multiple containers
  - Benefit: Better scaling on many cores

## Future Improvements

Planned enhancements to extend functionality:

- Additional Classifier Types
  - Video frame analysis
  - Audio fingerprinting
  - Metadata extraction

- Classification Export
  - JSON/XML formats
  - Database integration
  - Cloud storage sync

- Remote Control
  - Mobile app integration
  - Web interface
  - Network control protocol

- Analysis Tools
  - Classification visualization
  - Pattern discovery
  - Suggestion system
