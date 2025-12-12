# Large Text Viewer - User Guide

## Building and Running

### Development Mode
```bash
cargo run
```

### Release Mode (Optimized)
```bash
cargo run --release
```

### Build Executable
```bash
cargo build --release
```
The executable will be in `target/release/large-text-viewer.exe`

## Features Implemented

### ✅ Core Functionality
- **Virtual Scrolling**: Only loads visible portions of files into memory
- **Fast File Opening**: Opens files of any size instantly using memory-mapped files
- **Memory Efficient**: Handles GB+ files without loading entire content
- **Line Numbers**: Toggle-able line numbers display

### ✅ Viewing Options
- **Multiple Encodings**: Supports UTF-8, ASCII, UTF-16 LE/BE, Windows-1252, ISO-8859-1
- **Auto-detect Encoding**: Automatically detects file encoding when opening
- **Wrap Mode**: Toggle line wrapping (View menu)
- **Font Customization**: Adjustable font size (8-32pt) via View menu
- **Dark/Light Themes**: Switch between dark and light themes
- **Line Numbers**: Show/hide line numbers

### ✅ Search & Navigation
- **Fast Search**: Efficient searching through large files
- **Regex Support**: Toggle regex mode for pattern matching
- **Go to Line**: Jump directly to any line number
- **Find Next/Previous**: Navigate through search results with counter

### ✅ Advanced Features
- **Tail Mode**: Auto-refresh for log files (watches file changes)
- **File Info**: Display file size, encoding, line count

## How to Use

### Opening a File
1. Click **File → Open** in the menu bar
2. Select any text file (can be very large)
3. The file will open instantly with auto-detected encoding

### Searching
1. Type your search query in the Search field
2. Check "Use Regex" in Search menu for regular expressions
3. Click **🔍 Find** or press Enter
4. Use **⬆ Previous** and **⬇ Next** to navigate results
5. Results counter shows current position

### Navigation
- **Go to Line**: Enter a line number and click "Go" or press Enter
- **Scroll**: Use mouse wheel or scrollbar to navigate
- **Search Navigation**: Use Previous/Next buttons to jump between matches

### View Options
- **File → File Info**: Shows detailed file information
- **View → Word Wrap**: Toggle line wrapping
- **View → Line Numbers**: Show/hide line numbers
- **View → Dark Mode**: Switch between dark/light themes
- **View → Font Size**: Adjust font size (8-32pt)
- **View → Select Encoding**: Change file encoding manually

### Tail Mode (Log Files)
1. Open a log file
2. Enable **Tools → Tail Mode**
3. File will auto-refresh when changes are detected
4. Automatically scrolls to bottom on updates

## Performance Notes

### Memory Efficiency
- Uses memory-mapped files (no loading entire file into RAM)
- Only renders visible lines + buffer
- Handles multi-GB files efficiently

### Large File Indexing
- Files < 100 MB: Full line indexing
- Files > 100 MB: Sample-based indexing (faster, approximate line counts)

### Search Performance
- Max 10,000 results per search
- Regex patterns may be slower on very large files
- Uses efficient string matching algorithms

## Keyboard Shortcuts
- **Enter** in Search field: Perform search
- **Enter** in Go to Line: Jump to line
- Mouse wheel: Scroll through content

## Technical Details

### Architecture
- **Language**: Rust
- **GUI Framework**: egui/eframe (immediate mode GUI)
- **File I/O**: memmap2 (memory-mapped files)
- **Encoding**: encoding_rs (character encoding)
- **File Watching**: notify crate (tail mode)
- **Search**: regex crate (pattern matching)

### Supported Encodings
- UTF-8 (with BOM detection)
- UTF-16 LE (Little Endian)
- UTF-16 BE (Big Endian)
- Windows-1252
- ISO-8859-1

## Troubleshooting

### File Won't Open
- Check file permissions
- Ensure file exists and is readable
- Try selecting a different encoding

### Search Not Working
- Verify regex syntax if using regex mode
- Large files may take longer to search
- Check search results counter

### Performance Issues
- Disable word wrap for very long lines
- Reduce font size if rendering is slow
- Close file info window when not needed

## Future Enhancements (Not Yet Implemented)
- Copy/paste text selection (UI framework limitation)
- Bookmarks for quick navigation
- Column selection mode
- Hex viewer mode
- Compare two files side-by-side

## Building from Source

### Prerequisites
- Rust 1.70+ (install from https://rustup.rs/)
- Windows, macOS, or Linux

### Dependencies
All dependencies are managed by Cargo and will be downloaded automatically:
- eframe/egui: GUI framework
- memmap2: Memory-mapped files
- encoding_rs: Character encoding
- notify: File system watching
- regex: Pattern matching
- rfd: File dialogs
- anyhow: Error handling

### Build Commands
```bash
# Clone or navigate to project directory
cd large-text-viewer

# Build debug version
cargo build

# Build release version (optimized)
cargo build --release

# Run tests (if any)
cargo test

# Clean build artifacts
cargo clean
```

## License
See LICENSE file for details.

## Contributing
Contributions welcome! Please ensure code compiles without errors before submitting.
