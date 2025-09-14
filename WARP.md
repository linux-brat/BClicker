# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Project Overview

BClicker Professional is a Rust-based auto-clicker application with a TUI (Terminal User Interface) built using crossterm and tui-rs. The application provides professional auto-clicking functionality with global hotkey support, system tray integration, audio feedback, and statistics tracking.

**Technology Stack:**
- Language: Rust (2024 edition)
- UI Framework: tui-rs with crossterm backend
- Mouse Control: enigo crate
- Audio: rodio for sound effects
- Configuration: TOML serialization with serde
- System Tray: tray-item
- Notifications: notify-rust
- Platform: Windows-focused (with global hotkey support via Win32 API)

## Installation

### Prerequisites
- **Rust toolchain**: Install from [rustup.rs](https://rustup.rs/)
- **Git**: For cloning the repository
- **Windows**: Windows SDK for global hotkey functionality
- **Linux**: Audio development libraries (ALSA/PulseAudio)

### Install from GitHub

#### Windows
```powershell
# Clone the repository
git clone https://github.com/username/BClicker.git
cd BClicker

# Build and install
cargo build --release

# Run the application
.\target\release\BClicker.exe

# Optional: Add to PATH for system-wide access
# Copy target\release\BClicker.exe to a directory in your PATH
```

#### Linux
```bash
# Install audio development libraries (Ubuntu/Debian)
sudo apt update
sudo apt install libasound2-dev pkg-config

# For other distributions:
# Fedora: sudo dnf install alsa-lib-devel pkgconf
# Arch: sudo pacman -S alsa-lib pkgconf

# Clone the repository
git clone https://github.com/username/BClicker.git
cd BClicker

# Build and install
cargo build --release

# Run the application
./target/release/BClicker

# Optional: Install system-wide
sudo cp target/release/BClicker /usr/local/bin/
```

### Direct Cargo Installation
```bash
# Install directly from GitHub (if published)
cargo install --git https://github.com/username/BClicker.git

# Or install from crates.io (if published)
cargo install bclicker
```

## Common Development Commands

### Build and Run
```bash
# Build the project in debug mode
cargo build

# Build for release (optimized)
cargo build --release

# Run the application in debug mode
cargo run

# Run with release optimizations
cargo run --release
```

### Testing and Linting
```bash
# Run tests (if any exist)
cargo test

# Check code without building
cargo check

# Format code according to Rust standards
cargo fmt

# Run clippy for additional linting
cargo clippy

# Run clippy with strict settings
cargo clippy -- -D warnings
```

### Development Utilities
```bash
# Clean build artifacts
cargo clean

# Update dependencies
cargo update

# Show dependency tree
cargo tree

# Check for security vulnerabilities
cargo audit
```

## Architecture Overview

### Core Application Structure

The application follows a modular architecture with clear separation of concerns:

**Main Components:**
1. **App State Management** (`App` struct) - Central application state, configuration, and UI modes
2. **Event System** - Multi-threaded event handling for input, ticking, and quit signals
3. **Auto-Clicker Engine** - Dedicated thread for mouse clicking with precise timing
4. **UI Rendering** - TUI-based interface with dynamic content and help system
5. **System Integration** - Global hotkeys, system tray, and notifications

### Key Data Structures

**Configuration System:**
- `Config` struct handles all persistent settings
- `Statistics` struct tracks usage metrics across sessions
- `KeyCombo` struct represents global hotkey combinations
- Auto-saves to `bclicker_config.toml` in the current directory

**Threading Architecture:**
- **Main Thread**: UI rendering and input handling
- **Clicker Thread**: High-precision mouse clicking loop
- **Event Threads**: Separate threads for input capture and tick events
- **Hotkey Thread**: Windows-specific global hotkey monitoring
- **Audio Threads**: Spawned per-sound for non-blocking audio

### Input Mode System

The application uses a state machine for different input modes:
- `Normal` - Standard navigation and controls
- `EditingCps` - Custom CPS value input
- `SettingKeybind` - Capturing hotkey combinations
- `AwaitingKeybind` - Brief preparation state before capturing
- `ShowingHelp` - Help screen display

### Platform-Specific Features

**Windows Integration:**
- Global hotkey registration via Win32 API (`RegisterHotKey`)
- System message loop for hotkey detection
- Windows-specific virtual key code mapping

**Cross-Platform Considerations:**
- Mouse control works on all platforms via enigo
- System tray and notifications have fallback behavior
- Global hotkeys currently Windows-only with graceful degradation

## Configuration and Data Files

### Primary Configuration File
- **Location**: `bclicker_config.toml` (current directory)
- **Format**: TOML with nested sections
- **Auto-generated**: Creates default config if missing
- **Auto-saved**: Persists changes immediately

### Configuration Structure
```toml
cps_presets = [20, 30, 40, 50]  # Available CPS preset values
selected_preset = 2              # Currently selected preset index
custom_cps_value = 99           # User-defined CPS value
using_custom_cps = false        # Whether to use custom vs preset
selected_button = 0             # 0=Left, 1=Right mouse button
sound_enabled = false           # Audio feedback toggle

[toggle_keybind]                # Global hotkey configuration
mods = 6                        # Modifier bitmask (1=Shift, 2=Ctrl, 4=Alt)
key = "B"                       # Key character or function key

[statistics]                    # Usage tracking
total_clicks = 0                # All-time click count
session_clicks = 0              # Current session clicks
total_sessions = 0              # Number of application launches
last_session_start = 0          # Unix timestamp
session_duration = 0            # Session length in seconds
```

## Development Considerations

### Performance Characteristics
- **Click Precision**: Microsecond-accurate timing using `Duration::from_micros(1_000_000 / cps)`
- **UI Responsiveness**: 60 FPS rendering loop with conditional redrawing
- **Memory Efficiency**: Minimal allocations in hot paths, Arc/Mutex for shared state
- **CPU Usage**: Adaptive sleep timing based on UI visibility

### Thread Safety Patterns
- `Arc<AtomicBool>` for simple boolean flags (clicker running, UI visibility)
- `Arc<Mutex<T>>` for complex shared data (statistics, configuration)
- MPSC channels for event communication between threads
- Lock contention minimized through brief critical sections

### Error Handling Strategy
- Graceful degradation for system integration features
- Configuration corruption handled with default fallback
- Non-critical errors logged but don't crash application
- Proper cleanup on exit with terminal restoration

### Key Extension Points

**Adding New Features:**
1. **New Input Modes**: Extend `InputMode` enum and add handlers in `handle_input()`
2. **Additional Statistics**: Add fields to `Statistics` struct with migration logic
3. **Audio Enhancements**: Extend `AudioManager` with new sound methods
4. **Platform Support**: Implement platform-specific modules for hotkeys/tray

**UI Customization:**
- Theme system already in place with `Theme` struct
- Modular widget rendering in `draw_ui()` function
- Help system supports scrolling and formatted content
- Status bar easily extensible with new information

### Common Patterns

**Configuration Changes:**
```rust
// Always save config after modifications
app.config.some_setting = new_value;
app.save_config();
app.needs_redraw = true;  // Trigger UI update
```

**Shared State Updates:**
```rust
// Safe access to shared statistics
if let Ok(mut stats) = app.stats_tracker.lock() {
    stats.some_field += 1;
    // Lock automatically released
}
```

**Event-Driven Updates:**
```rust
// Mark redraw needed for next frame
app.needs_redraw = true;
// Only redraw when UI is visible and needs update
```

## Troubleshooting

### Common Build Issues
- **Missing Windows SDK**: Global hotkeys require Windows development headers
- **Audio Dependencies**: rodio may need system audio libraries on Linux
- **Terminal Compatibility**: Some terminals may not support all TUI features

### Runtime Considerations
- **Hotkey Conflicts**: Global hotkeys may conflict with other applications
- **Permission Issues**: Some antivirus software may flag mouse automation
- **Terminal Encoding**: Unicode characters in UI require UTF-8 terminal support

### Development Tips
- Use `cargo run` for development with debug info and faster compilation
- The application handles terminal cleanup automatically on exit
- Configuration file is human-readable and can be manually edited
- Statistics are preserved across application restarts