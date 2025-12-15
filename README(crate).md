# large-text-viewer
A cross-platform Rust GUI application for viewing and editing multi-GB text files efficiently using memory-mapped I/O and parallel processing.

## Features

### Core Functionality
- **Virtual Scrolling**: Only loads visible portions of the file into memory
- **Fast File Opening**: Opens files of any size instantly
- **Memory Efficient**: Uses memory-mapped files for optimal performance
- **Line Numbers**: Displays line numbers for easy navigation

### Viewing Options
- **Multiple Encodings**: UTF-8, ASCII, UTF-16 LE/BE, ISO-8859-1, and more
- **Wrap Mode**: Toggle line wrapping
- **Font Customization**: Adjustable font size and family
- **Dark/Light Themes**: Theme support
- **Highlight Text & Copy Paste**: Highlight text so that user can just copy and paste like usual

### Search & Navigation
- **Fast Search**: Efficient searching through large files
- **Regex Support**: Search with regular expressions
- **Go to Line**: Jump directly to any line number
- **Find Next/Previous**: Navigate through search results

### Advanced Features
- **Tail Mode**: Auto-refresh for log files (watches file changes)
- **File Info**: Display file size, encoding, line count estimates

## Technology Stack

- **Language**: Rust (for maximum performance and memory safety)
- **GUI Framework**: `egui`
- **File I/O**: `memmap2` for memory-mapped file access
- **Encoding**: `encoding_rs` for character encoding support
- **File Watching**: `notify` crate for tail mode

## Architecture

### Key Components

1. **FileReader**: Memory-mapped file reader that loads chunks on demand
2. **LineIndexer**: Builds an index of line positions for fast navigation
3. **VirtualScroller**: Renders only visible lines + next window of lines as buffer
4. **SearchEngine**: Efficient search with optional regex support
5. **EncodingDetector**: Detects and converts file encodings

### Performance Optimizations

- Lazy loading of file content and view point based rendering
- Efficient line indexing (sample-based for very large files)
- Web Workers for search operations
- Chunked reading strategy
- Cache visible content

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

