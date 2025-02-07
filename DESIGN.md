# Classi-cine Design Document

## Overview
Classi-cine is a video file classifier that uses machine learning and interactive user feedback through VLC media player to help organize video collections. It combines multiple classification approaches including file characteristics and content-based features.

## Core Components

### 1. Classification System
The system uses multiple classifiers working in parallel:
- File Size Classifier: Scores based on file size using logarithmic scaling
- Directory Size Classifier: Scores based on number of files in directories
- Naive Bayes Classifier: Learns from text features extracted from filenames

All classifier scores are normalized and combined to rank candidates.

### 2. Text Processing Pipeline
- Normalization: Standardizes filenames for consistent processing
- Tokenization: Breaks normalized text into tokens using pair encoding
- N-gram Generation: Creates n-grams from tokens for feature extraction
- Bloom Filter: Efficient storage and lookup of n-gram features

### 3. VLC Integration
- HTTP Interface: Controls VLC via its HTTP API
- Classification Controls:
  - Pause = Positive classification
  - Stop = Negative classification
- Process Management: Handles VLC lifecycle and status monitoring

### 4. Playlist Management
- M3U Format: Stores classifications in standard playlist format
- Maintains separate positive and negative classifications
- Supports incremental updates

## Data Flow

1. File Collection
   - Walks specified directories
   - Filters by video file extensions
   - Excludes previously classified files

2. Text Processing
   - Normalizes filenames
   - Builds tokenizer from all paths
   - Generates n-grams
   - Identifies frequent n-grams

3. Classification
   - Initializes classifiers
   - Processes entries to establish bounds
   - Calculates and normalizes scores
   - Sorts entries by combined score

4. Interactive Loop
   - Presents highest scoring candidate
   - Launches VLC for preview
   - Captures user classification
   - Updates playlist and training data
   - Reprocesses remaining entries

## Configuration
Command line arguments control:
- Input directories
- Playlist location
- Video file extensions
- VLC display options
- Network ports
- Logging level

## Performance Considerations
- Thread priority management
- Parallel processing where applicable
- Bloom filters for efficient feature lookup
- Sharded data structures for concurrent access

## Future Improvements
- Additional classifier types
- Alternative media players
- Classification export formats
- Remote control interface
- GPU acceleration for feature extraction
