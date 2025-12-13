# Architecture and Design Document

## Overview

This document describes the architecture and design decisions for the Large Text File Viewer application.

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Main GUI                      │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │   Toolbar   │  │  Search Bar  │  │ Replace Bar  │       │
│  └─────────────┘  └──────────────┘  └──────────────┘       │
│  ┌─────────────────────────────────────────────────────┐   │
│  │           Viewport (50 lines at a time)              │   │
│  └─────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────┐   │
│  │               Navigation Controls                     │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    Core Components                           │
│                                                               │
│  ┌──────────────────┐  ┌──────────────────┐                │
│  │  FileHandler     │  │  Search Module   │                │
│  │  - memmap2       │  │  - rayon         │                │
│  │  - Line indexing │  │  - Parallel scan │                │
│  │  - Viewport mgmt │  │  - Regex support │                │
│  └──────────────────┘  └──────────────────┘                │
│                                                               │
│  ┌──────────────────┐                                        │
│  │  Editor Module   │                                        │
│  │  - rayon         │                                        │
│  │  - Replace ops   │                                        │
│  │  - File I/O      │                                        │
│  └──────────────────┘                                        │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                    File System                               │
│             (Memory-mapped file via memmap2)                 │
└─────────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### 1. Memory-Mapped I/O (memmap2)

**Decision**: Use `memmap2` for file access instead of traditional buffered I/O.

**Rationale**:
- **Efficiency**: The OS handles paging automatically, allowing efficient access to files larger than RAM
- **Performance**: Direct memory access is faster than repeated system calls
- **Simplicity**: No need to manage buffer sizes or complex caching strategies
- **Scalability**: Works seamlessly with files >4GB

**Implementation**:
```rust
pub struct FileHandler {
    mmap: Arc<Mmap>,           // Memory-mapped file
    line_offsets: Arc<Vec<usize>>,  // Index of line positions
    modified_lines: Arc<RwLock<HashMap<usize, String>>>,  // In-memory edits
}
```

### 2. Line Offset Indexing

**Decision**: Build a complete index of line offsets on file open.

**Rationale**:
- **Fast Random Access**: Jump to any line in O(1) time
- **Memory Efficient**: Only stores offsets (8 bytes per line)
- **Viewport Optimization**: Quickly retrieve any range of lines
- **One-time Cost**: Index built once during file opening

**Trade-offs**:
- Initial loading time increases with file size
- Memory overhead of ~8 bytes per line (acceptable for 10GB file = ~80MB index for 10M lines)

### 3. Viewport Rendering

**Decision**: Render only visible lines (default: 50 lines).

**Rationale**:
- **Memory Efficient**: Constant memory usage regardless of file size
- **Responsive UI**: Fast rendering even for huge files
- **Practical**: Users typically view one screen at a time

**Implementation**:
```rust
pub fn get_viewport_lines(&self, start_line: usize, count: usize) -> Vec<String> {
    let total_lines = self.total_lines();
    let end_line = (start_line + count).min(total_lines);
    
    (start_line..end_line)
        .filter_map(|i| self.get_line(i))
        .collect()
}
```

### 4. Parallel Search with Rayon

**Decision**: Use `rayon` to parallelize search across file chunks.

**Rationale**:
- **Performance**: Utilizes multiple CPU cores effectively
- **Simplicity**: Rayon's API makes parallel iteration straightforward
- **Scalability**: Search time scales with number of cores, not just file size

**Implementation**:
```rust
let results: Vec<Vec<usize>> = chunks
    .par_iter()
    .map(|&start| {
        // Search chunk in parallel
        search_chunk(start, chunk_size)
    })
    .collect();
```

**Chunk Size**: 1000 lines per chunk
- Small enough for good parallelization
- Large enough to avoid excessive overhead

### 5. Copy-on-Write Replace Strategy

**Decision**: Use atomic file replacement instead of true in-place editing.

**Rationale**:
- **Safety**: Original file remains untouched until operation completes
- **Atomicity**: Either succeeds completely or fails without corruption
- **Simplicity**: Easier to implement than complex in-place algorithms
- **Reliability**: No partial edits in case of crashes

**Implementation**:
1. Create temporary file
2. Process chunks in parallel
3. Write results to temp file
4. Atomically rename temp file to original

**Trade-offs**:
- Requires disk space for temporary file (same size as original)
- Not true in-place (but safer and simpler)

### 6. Thread-Safe Line Modifications

**Decision**: Use `Arc<RwLock<HashMap>>` for tracking line edits.

**Rationale**:
- **Concurrency**: Multiple threads can read simultaneously
- **Safety**: Write lock ensures consistency
- **Flexibility**: Tracks individual line changes without rewriting entire file

**Implementation**:
```rust
modified_lines: Arc<RwLock<HashMap<usize, String>>>
```

### 7. Regex vs Literal Search

**Decision**: Auto-detect search type based on regex validity.

**Rationale**:
- **User-Friendly**: No need to specify search mode
- **Performance**: Use faster literal search when possible
- **Flexibility**: Support both patterns seamlessly

**Implementation**:
```rust
let is_regex = Regex::new(query).is_ok();
if is_regex {
    search_regex(...)
} else {
    search_literal(...)
}
```

## Performance Characteristics

### Memory Usage

| File Size | Index Size | Viewport Memory | Total Memory |
|-----------|-----------|----------------|--------------|
| 1 GB      | ~8 MB     | ~10 KB         | ~8 MB        |
| 10 GB     | ~80 MB    | ~10 KB         | ~80 MB       |
| 100 GB    | ~800 MB   | ~10 KB         | ~800 MB      |

### Search Performance

With 8 cores and 1000-line chunks:
- **Throughput**: ~8000 lines processed in parallel per iteration
- **Speedup**: Near-linear with core count for large files
- **Latency**: Minimal for files <1M lines, scales logarithmically

### Replace Performance

- **Parallel Processing**: Each chunk processed independently
- **I/O Bound**: Ultimately limited by disk write speed
- **Safety**: No performance penalty for atomic operations

## Future Enhancements

### Potential Improvements

1. **Lazy Line Indexing**
   - Build index incrementally as file is explored
   - Faster startup for very large files
   - Trade-off: Slower first access to distant lines

2. **Virtual Scrolling**
   - More sophisticated viewport management
   - Smooth scrolling animations
   - Pre-fetch adjacent viewports

3. **Incremental Search**
   - Start searching before user finishes typing
   - Cancel previous searches when new query arrives
   - Progressive result display

4. **Undo/Redo**
   - Command pattern for edit operations
   - In-memory history for recent changes
   - Disk-based history for large modifications

5. **Syntax Highlighting**
   - Detect file type
   - Apply appropriate syntax highlighting
   - Use tree-sitter for parsing

6. **Split View**
   - Multiple viewports for same file
   - Compare different sections
   - Independent scrolling

## Testing Strategy

### Unit Tests

- File handler operations (basic, viewport, modifications)
- Search functionality (literal, regex)
- Replace operations (literal, regex)
- Edge cases (empty files, single line, no newline at EOF)

### Integration Tests

- End-to-end file operations
- GUI interactions (would require UI testing framework)
- Large file handling (>1GB files)

### Performance Tests

- Benchmark search on various file sizes
- Measure memory usage growth
- Test parallel efficiency

### Security Considerations

- Input validation for regex patterns
- Safe file system operations
- No buffer overflows (Rust's memory safety)
- Atomic file operations prevent corruption

## Conclusion

This architecture provides a solid foundation for efficiently viewing and editing multi-GB text files. The combination of memory-mapped I/O, viewport rendering, and parallel processing ensures good performance while maintaining low memory usage and a responsive user interface.