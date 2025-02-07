# Classi-cine Design Document

## Overview
Classi-cine is an intelligent video organization tool that combines multiple machine learning approaches 
with interactive user feedback to efficiently sort large video collections. It addresses common challenges
in managing video libraries:
- Time-consuming manual organization
- Inconsistent naming conventions
- Need to preview content before classification
- Desire to learn from user preferences over time

The system uses VLC media player for previews and classification input, making it immediately familiar
to users while providing a robust foundation for video playback across formats and platforms.

## Core Components

### 1. Multi-Classifier System
The system employs multiple complementary classifiers to capture different aspects of video organization:

- File Size Classifier
  - Why: File size often correlates with video quality and content type
  - How: Uses logarithmic scaling to handle wide size ranges naturally
  - Benefit: Helps distinguish between different quality versions or content types

- Directory Size Classifier
  - Why: Directory structure often reflects meaningful organization
  - How: Analyzes number of files in directories to identify patterns
  - Benefit: Learns from existing manual organization efforts

- Naive Bayes Classifier
  - Why: Filenames often contain valuable semantic information
  - How: Learns from n-gram patterns in normalized filenames
  - Benefit: Captures naming conventions and content indicators

The scores from all classifiers are normalized and combined, allowing the system to:
- Balance multiple factors in decision making
- Remain robust when individual signals are weak
- Adapt to different organization strategies

### 2. Text Processing Pipeline
A sophisticated pipeline processes filenames to extract meaningful features:

- Normalization
  - Why: Raw filenames vary widely in format and style
  - How: Standardizes case, spacing, and special characters
  - Benefit: Enables consistent pattern recognition

- Pair Encoding Tokenization
  - Why: Traditional word splitting often fails with filenames
  - How: Learns common character pairs to identify word boundaries
  - Benefit: Handles arbitrary naming conventions and languages

- N-gram Generation
  - Why: Individual tokens may be too granular
  - How: Creates overlapping sequences of tokens
  - Benefit: Captures phrases and context

### 3. VLC Integration
Seamless integration with VLC provides an efficient classification workflow:

- HTTP Interface
  - Why: Enables programmatic control without VLC modifications
  - How: Uses VLC's built-in HTTP server
  - Benefit: Reliable cross-platform operation

- Simple Controls
  - Why: Classification should be quick and intuitive
  - How: Maps pause/stop to positive/negative classifications
  - Benefit: Users can classify while naturally previewing content

- Process Management
  - Why: VLC instances need careful handling
  - How: Monitors process state and handles cleanup
  - Benefit: Prevents resource leaks and zombie processes

### 4. Playlist Management
Uses M3U playlists as a robust storage format:

- Standard Format
  - Why: Universal compatibility
  - How: Uses plain text with simple markup
  - Benefit: Results work with any media player

- Separate Classifications
  - Why: Different use cases need different organizations
  - How: Maintains distinct positive/negative lists
  - Benefit: Supports multiple organization schemes

- Incremental Updates
  - Why: Classification is often an ongoing process
  - How: Appends new classifications immediately
  - Benefit: Progress is never lost

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
