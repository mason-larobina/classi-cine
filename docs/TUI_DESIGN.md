# TUI Design Plan for Classi-Cine

## Overview

This document outlines the design for a visually rich Terminal User Interface (TUI) for classi-cine, transforming it from a basic CLI tool into a sophisticated, interactive interface that rivals modern monitoring tools like btop while maintaining all the powerful ML classification capabilities.

## Current Architecture Analysis

Classi-cine is a sophisticated ML-powered video playlist builder with:
- **Multi-threaded VLC integration** via HTTP interface
- **Multiple classifiers**: Naive Bayes, file size, directory size, file age
- **Interactive classification loop** with scoring and visualization
- **Current visualization**: Simple ASCII plots via `textplots` crate

## TUI Framework Choice: **Ratatui + Crossterm**

- **Ratatui** (successor to tui-rs) - Modern, actively maintained TUI framework
- **Crossterm** - Cross-platform terminal backend
- **Component-based architecture** with React-like patterns

## Layout Design

### Main View Layout (Interactive Classification Interface)

```
┌─ File List (50%) ─────────────────┬─ Selected File Details (50%) ──┐
│   0.89 │ terminator2.mp4          │ Full Path:                     │
│   0.76 │ matrix.mkv               │ /movies/action/terminator2.mp4 │
│   0.63 │ john_wick.mp4            │                                │
│ > 0.58 │ avatar.mp4               │ Normalized:                    │
│   0.41 │ deadpool.avi             │ movies action terminator2 mp4  │
│   0.28 │ comedy_special.mkv       │                                │
│   0.19 │ tutorial.mp4             │ Tokenized:                     │
│   0.12 │ boring_doc.avi           │ ["movies", "action",           │
│   0.08 │ documentary.avi          │  "terminator", "2", "mp4"]     │
│                                   │                                │
│                                   │ Top 100 N-grams:               │
│                                   │ ┌────────────────────────────┐ │
│                                   │ │ action        ████████ 0.85│ │
│                                   │ │ terminator    ███████░ 0.71│ │
│                                   │ │ movie         ██████░░ 0.62│ │
│                                   │ │ sci           █████░░░ 0.54│ │
│                                   │ │ fi            █████░░░ 0.52│ │
│                                   │ │ 1991          ████░░░░ 0.48│ │
│                                   │ │ classic       ████░░░░ 0.45│ │
│                                   │ │ arnold        ████░░░░ 0.43│ │
│                                   │ │ cameron       ███░░░░░ 0.41│ │
│                                   │ │ sequel        ███░░░░░ 0.38│ │
│                                   │ │ ...                        │ │
│                                   │ │ boring        ░░░░███ -0.56│ │
│ Total: 156 files                  │ │ documentary   ░░░░██░ -0.43│ │
│ ✓ Positive: 42  ✗ Negative: 18    │ │ tutorial      ░░░░██░ -0.38│ │
│ ○ Pending: 96                     │ └────────────────────────────┘ │
└───────────────────────────────────┴────────────────────────────────┘
┌─ Help & Controls ──────────────────────────────────────────────────┐
│ [↑/↓] Navigate  [Enter] Select  [P] Positive  [N] Negative         │
│ [R] Re-classify  [Esc/Ctrl+C] Exit                                 │
└────────────────────────────────────────────────────────────────────┘
```

#### Interface Specifications

**Left Pane - File List Table:**
- Two-column table: Score | Filename
- Current selection highlighted with `>` and inverted colors
- Status indicators: `✓` (positive), `✗` (negative), `○` (pending), `?` (unscored)
- Score display: numeric 0.00-1.00 or `????` for unscored files
- Scrollable list with keyboard navigation (↑/↓)

**Right Pane - Selected File Details:**
- **Full Path**: Complete filesystem path to the selected file
- **Normalized**: Path converted to space-separated tokens for ML processing
- **Tokenized**: Array representation showing individual tokens after processing
- **Top 100 N-grams**: Scrollable list of most significant n-grams with:
  - N-gram text on left
  - Visual intensity bar (color-coded by positive/negative influence)
  - Numeric weight/score on right
  - Sorted by absolute value of influence

**Color Coding for N-grams:**
- Positive influence: Green gradient (█ = strongest positive)
- Negative influence: Red gradient (█ = strongest negative)
- Bar length represents absolute influence strength

## Interactive Features & User Workflows

### 1. **Real-time Classification Mode** (Enhanced Build Command)
- Live file queue with score indicators  
- Dynamic score updates as classification happens
- Color-coded entries (green=positive, red=negative, yellow=pending)
- Smooth animations for score changes

### 2. **Exploration Mode** (Enhanced Score Command)
- Sortable file browser with multiple sort criteria
- Interactive filtering by classifier scores
- Directory tree view with aggregated scores
- Search functionality with regex support

### 3. **Analysis Dashboard**
- Historical classification patterns
- Classifier performance metrics
- N-gram frequency analysis
- Export capabilities (JSON, CSV, M3U)

### 4. **VLC Integration Panel**
- Mini video preview (ASCII art thumbnails)
- Playback controls overlay
- Progress bar with seeking
- Volume/audio visualization

### 5. **Configuration Panel**
- Live parameter adjustment (bias values, thresholds)
- Theme switching (btop-inspired color schemes)
- Keyboard shortcut customization
- Export/import settings

## Visual Elements & Styling

### Color Schemes (btop-inspired)
- **Default**: Blue/cyan gradients with white text
- **Gruvbox**: Warm orange/brown palette  
- **Monokai**: Purple/pink accent colors
- **Solarized**: Light/dark variants
- **Matrix**: Green terminal aesthetic

### Rich Visual Components
- **Progress bars**: Multi-segment with color gradients
- **Sparklines**: Mini-charts for score trends  
- **Box plots**: Statistical distribution visualization
- **Heat maps**: Directory/file activity patterns
- **Gauges**: Circular progress indicators for scores
- **Tables**: Sortable with alternating row colors
- **Graphs**: Real-time scoring charts with legends

### Special Characters & Icons
- File type icons: 🎬📹🎞️📼
- Status indicators: ◆○●⬢⬡
- Progress: ▓▒░█▉▊▋▌▍▎▏
- Arrows/pointers: ▲▼◀▶←→↑↓
- Borders: ┌┐└┘├┤┬┴─│

## Implementation Roadmap

### Phase 1: Core TUI Framework
1. **Setup Ratatui + Crossterm dependencies**
2. **Create basic layout manager** with resizable panes
3. **Implement event handling** (keyboard, mouse, resize)
4. **Design component system** (similar to React components)

### Phase 2: Data Integration  
1. **Connect to existing App struct** without breaking CLI
2. **Create TUI mode flag** (`--tui` or `tui` subcommand)
3. **Stream data updates** from classification loop
4. **Implement state management** (Redux-like pattern)

### Phase 3: Interactive Features
1. **File queue component** with real-time updates
2. **Score visualization panels** with live charts
3. **VLC integration overlay** with playback controls
4. **Keyboard shortcuts** and help system

### Phase 4: Advanced Visualization
1. **Multi-threaded rendering** for smooth animations
2. **Chart components** (histograms, scatter plots, time series)
3. **Color themes** with user preferences
4. **Mouse interaction** (clicking, scrolling, dragging)

### Phase 5: Polish & Performance
1. **Optimize rendering** for large file sets
2. **Add configuration** system and persistence
3. **Error handling** and graceful degradation
4. **Documentation** and examples

## Technical Architecture

### Component Hierarchy
```rust
App
├── HeaderBar (title, stats, time)
├── MainLayout
│   ├── FileQueuePanel
│   │   ├── QueueList (scrollable)
│   │   ├── ProgressBar
│   │   └── Stats
│   ├── ScorePanels
│   │   ├── ClassifierScores (4 panels)
│   │   ├── CombinedScore
│   │   └── ConfidenceGauge
│   ├── FileDetails
│   │   ├── PathDisplay
│   │   ├── MetadataTable
│   │   └── TokenView
│   └── VlcPanel
│       ├── StatusDisplay
│       ├── Controls
│       └── ProgressBar
├── ScoreDistribution
└── FooterBar (help text, current mode)
```

### State Management
```rust
#[derive(Clone)]
pub struct TuiState {
    pub mode: AppMode, // Classification, Exploration, Analysis
    pub current_file: Option<Entry>,
    pub file_queue: Vec<Entry>,
    pub classifier_scores: HashMap<String, f64>,
    pub vlc_status: VlcStatus,
    pub selected_panel: PanelId,
    pub theme: ColorTheme,
    pub config: TuiConfig,
}
```

### Key Dependencies
```toml
[dependencies]
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
color-eyre = "0.6"
tui-textarea = "0.6"  # For text input components
```

### Integration Strategy

The TUI will be implemented as an optional mode that can be enabled via:
```bash
classi-cine build --tui <playlist.m3u> <directories...>
classi-cine score --tui <playlist.m3u> <directories...>
```

This preserves the existing CLI interface while adding the rich TUI experience for users who want it.

## Design Goals

1. **Visual Richness**: Match or exceed btop's visual appeal with color, charts, and smooth animations
2. **Functional Completeness**: Support all existing classi-cine features through the TUI
3. **Real-time Updates**: Live data streaming for an engaging classification experience
4. **Accessibility**: Keyboard navigation, color-blind friendly themes, terminal compatibility
5. **Performance**: Smooth operation even with large file sets (1000+ videos)
6. **Extensibility**: Modular design for adding new visualization components

This TUI design will transform classi-cine into a modern, engaging tool that makes machine learning-powered video classification both powerful and enjoyable to use.
