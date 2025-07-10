# Ratatui Migration and TUI Implementation Design Document

## 1. Introduction

### Purpose
This document outlines the design for migrating the Classi-Cine application to use Ratatui for a Terminal User Interface (TUI). The TUI will provide interactive views for playlist entries, pending classification entries, classifier states (e.g., ngram scores), tokenizer vocabulary, and other internal states. This enhances user introspection and control during the classification process.

The core classification workflow remains unchanged: VLC is used to play videos and capture user feedback (positive/negative/skipped classifications). The TUI will run alongside VLC, allowing users to inspect data before, during, or after classification sessions.

### Goals
- Interactive inspection of application state without interrupting the classification loop.
- Modular views for different components (e.g., playlists, entries, classifiers).
- Seamless integration with existing Rust codebase (e.g., App struct, classifiers, tokenizer).
- Responsive TUI with keyboard navigation and event handling.
- Maintain performance for large datasets (e.g., thousands of entries or ngrams).

### Non-Goals
- Replacing VLC entirely; VLC remains the primary tool for video playback and classification input.
- Full GUI replacement; this is a TUI migration focused on terminal-based interaction.
- Real-time video rendering in the TUI.

## 2. Overall Architecture

### High-Level Components
The TUI will be built using Ratatui (a Rust library for building rich terminal user interfaces) and Crossterm for backend terminal handling. The architecture integrates with the existing `App` struct in `src/main.rs`.

- **Main Loop**: A central event loop handles user input (keyboard events), updates the UI state, and renders frames. It will run in a separate thread or asynchronously to avoid blocking the classification process.
- **State Management**: Extend the `App` struct with TUI-specific state (e.g., current tab, selected entry, view modes).
- **Views/Tabs**: Use Ratatui's widgets (e.g., Tabs, Tables, Lists, Charts) to create tabbed interfaces for different sections.
- **Integration Points**:
  - Access to `App`'s fields like `entries`, `playlist`, `naive_bayes`, `tokenizer`.
  - Trigger VLC playback from the TUI (e.g., on keypress for the current entry).
  - Update classifications in real-time based on VLC feedback.

### Dependencies
- Add `ratatui` and `crossterm` to `Cargo.toml`.
- Optionally, `tui` crates for advanced widgets if needed (e.g., for charts similar to existing `textplots`).

## 3. User Interface Layout

### Layout Structure
The TUI will use a vertical split layout:
- **Top Bar**: Application title, current mode (e.g., "Classification Mode"), and tabs.
- **Main Area**: Tab-specific content (e.g., tables, lists, details panes).
- **Bottom Bar**: Status messages, key bindings help, and progress indicators (e.g., "Entries left: 123").

### Tabs/Views
1. **Playlist View**:
   - Displays classified entries from `playlist.entries()`.
   - Table with columns: Type (Positive/Negative), Path, Score (if applicable).
   - Filtering/sorting by type or score.
   - Selection allows viewing details (e.g., tokens, ngrams).

2. **Pending Entries View**:
   - Lists unclassified entries from `App.entries`.
   - Table with columns: Path, File Size, Dir Size, Naive Bayes Score, Total Score.
   - Highlights the next entry for classification (based on sorting in `calculate_scores_and_sort_entries`).
   - Upcoming entries preview (e.g., top 10 by score).

3. **Classifier State View**:
   - Sub-views for each classifier (e.g., NaiveBayes, FileSize, DirSize).
   - For NaiveBayes: Table of ngrams with positive/negative counts, log probabilities, scores (from `ngram_score`).
   - Search/filter for specific ngrams.
   - Visualization of score distributions (migrate from `textplots` to Ratatui charts).

4. **Tokenizer View**:
   - Displays vocabulary from `tokenizer.token_map()` (e.g., token ID, string representation).
   - List of merges from `merges` vec.
   - For a selected entry: Show tokenized path, pairs, and ngrams.
   - Bloom filter stats if relevant.

5. **Details Pane** (Shared Across Tabs):
   - On selection: Show expanded info like `display_entry_details` (filename, tokens, top ngrams with scores).
   - Integrated with `get_classifiers()` for per-classifier scores.

### Navigation and Controls
- Keyboard-driven:
  - Tab switching: Left/Right arrows or numbers (1-5).
  - Selection: Up/Down arrows, Enter for details.
  - Actions: 'p' to play selected entry in VLC, 'q' to quit, 'r' to refresh scores.
  - Search: '/' to enter search mode.
- Modes: Normal (navigation), Inspect (detailed view), Search.

## 4. Integration with Existing Code

### Modifications to App
- Add TUI state fields to `App` (e.g., `current_tab: usize`, `selected_entry: Option<usize>`).
- New methods:
  - `init_tui()`: Set up Ratatui backend and event loop.
  - `render_frame()`: Draw the current UI state using Ratatui widgets.
  - `handle_event()`: Process keyboard inputs and update state.
- In `classification_loop()`: Instead of printing to console, update TUI views. Pause for user input if needed, then trigger VLC.

### VLC Integration
- TUI triggers VLC via `play_file_and_get_classification` on keypress (e.g., 'p' on selected entry).
- While VLC runs, TUI can show a "Classifying..." overlay or continue running in background.
- On classification return: Update `process_classification_result`, refresh TUI views (e.g., move entry to playlist tab, recalculate scores).

### Data Flow
- Periodic refresh: After each classification or on 'r', call `calculate_scores_and_sort_entries` and update views.
- Real-time updates: Use channels (e.g., mpsc) for async updates from classification thread to TUI thread.

## 5. Event Handling and Rendering

### Event Loop
- Use Crossterm for polling events (keys, resizes).
- Tick-based: Render every 250ms or on event.
- Handle resize events to adjust layouts dynamically.

### Rendering
- Use Ratatui's `Frame` for drawing.
- Widgets: `Tabs`, `Table`, `List`, `Paragraph` for details, `Chart` for distributions (replacing `textplots`).
- Styles: Use colors for positive/negative (green/red), highlights for selections.

## 6. Potential Challenges and Mitigations

- **Performance**: Large tables (e.g., thousands of ngrams) – Use lazy loading or pagination in tables.
- **Concurrency**: TUI and classification loop – Run TUI in main thread, classification in spawned threads with channels.
- **Terminal Compatibility**: Test with different terminals; use Ratatui's backend features.
- **State Synchronization**: Use Mutex/Arc for shared access to `App` fields.
- **Migration Effort**: Start with minimal TUI (e.g., just pending entries), iteratively add tabs.
- **Testing**: Add unit tests for UI state transitions; manual testing for rendering.

## 7. Implementation Plan

1. Add dependencies and set up basic TUI skeleton in `main.rs`.
2. Implement tab navigation and basic views (Playlist, Pending).
3. Integrate with classifiers and tokenizer for detailed views.
4. Migrate visualizations from `textplots` to Ratatui charts.
5. Hook into classification loop, test end-to-end.
6. Polish: Add search, filters, error handling in TUI.

This design provides a flexible, introspective TUI while preserving the application's core functionality.
