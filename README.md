# large-text-viewer

A cross-platform Rust GUI application for viewing and editing multi-GB text files efficiently using memory-mapped I/O and parallel processing.

## Features

### 1. **Viewport Rendering with Memory-Mapped I/O**
- Uses `memmap2` for efficient memory-mapped file I/O
- Loads and renders only visible lines from the viewport
- Ensures low memory usage even for files >4GB
- Fast random access to any line in the file

### 2. **Multithreaded Search**
- Parallel search across file chunks using `rayon`
- Supports both literal string search and regex patterns
- Automatically detects and uses appropriate search method
- Efficiently processes large files by dividing into chunks

### 3. **Multithreaded Replace**
- Safe in-place editing with atomic file replacement
- Copy-on-write approach for size-changing replacements
- Parallel processing of file chunks for fast replacements
- Supports both literal and regex-based replacements

### 4. **GUI Features**
- Built with `iced` framework for cross-platform support
- File open dialog for easy file selection
- Search navigation (next/previous match)
- Replace all functionality
- Line-by-line editing capability
- Viewport navigation (page up/down, jump to top)
- Status messages for user feedback

## Building

```bash
cargo build --release
```

## Running

```bash
cargo run --release
```

Or run the compiled binary:

```bash
./target/release/large-text-viewer
```

## Usage

1. **Open a File**: Click "Open File" button to select a text file
2. **Navigate**: Use "Page Up", "Page Down", and "Top" buttons to navigate through the file
3. **Search**: Enter a search query (literal or regex) and use "Next"/"Previous" to navigate results
4. **Replace**: Enter replacement text and click "Replace All" to replace all occurrences
5. **Save**: Click "Save File" to persist any changes

## Architecture

### Components

- **`file_handler.rs`**: Memory-mapped file handling with efficient line indexing
- **`search.rs`**: Parallel search implementation using rayon
- **`editor.rs`**: Multithreaded replace and file editing operations
- **`main.rs`**: GUI application using iced framework

### Performance Considerations

- **Memory Usage**: Only the viewport (default 50 lines) is rendered at a time
- **Search Performance**: Files are divided into chunks (1000 lines) and processed in parallel
- **File Safety**: All write operations use atomic file replacement to prevent data loss

## Technical Details

### Memory-Mapped I/O
The application uses `memmap2` to map files into memory, allowing the OS to handle paging automatically. This provides:
- Fast random access to any part of the file
- Minimal memory overhead
- Efficient handling of files larger than available RAM

### Parallel Processing
Both search and replace operations use `rayon` for parallel processing:
- Files are divided into chunks
- Each chunk is processed independently
- Results are aggregated efficiently

### GUI Framework
The `iced` framework provides:
- Cross-platform support (Windows, macOS, Linux)
- Reactive UI updates
- Async command handling
- Modern, performant rendering

## Testing

Run the test suite:

```bash
cargo test
```

The test suite includes:
- File handler tests (basic operations, viewport, line modification)
- Search tests (literal and regex search)
- Editor tests (replace operations, file saving)

## License

MIT

