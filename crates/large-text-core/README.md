# large-text-core

Core library for handling large text files search and replace efficiently. This crate provides the backend functionality for the Large Text Viewer application.

## Features

*   **Memory Mapping**: Uses `memmap2` for efficient file access without loading the entire file into RAM.
*   **Encoding Support**: Handles various text encodings (UTF-8, UTF-16, Windows-1252, etc.) using `encoding_rs`.
*   **Fast Indexing**: Indexes line offsets for quick random access to any line in the file.
*   **Search Engine**: Supports plain text and regex searching with multi-threaded processing.
*   **Efficient Replacement**: Performs search and replace operations, with optimizations for in-place replacements when lengths match.

## Modules

### `file_reader`
Handles opening files via memory mapping and provides methods to read chunks of text with proper encoding decoding.

### `line_indexer`
Builds an index of line start offsets. For extremely large files, it can use sparse sampling to estimate line positions while keeping memory usage low.

### `search_engine`
Provides functionality to search for strings or regular expressions. It supports:
*   Counting total matches.
*   Fetching matches in chunks/pages.
*   Case-sensitive and case-insensitive search.

### `replacer`
Handles writing changes back to the file. It supports:
*   Single occurrence replacement.
*   Global search and replace.
*   In-place replacement optimization when the new text length matches the old text length.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
large-text-core = { path = "crates/large-text-core" }
```

## License

MIT
