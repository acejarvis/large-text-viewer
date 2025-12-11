use anyhow::{Context, Result};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// FileHandler manages memory-mapped file I/O for efficient large file handling
#[derive(Clone)]
pub struct FileHandler {
    mmap: Arc<Mmap>,
    line_offsets: Arc<Vec<usize>>,
    modified_lines: Arc<RwLock<HashMap<usize, String>>>,
}

impl FileHandler {
    /// Create a new FileHandler from a file path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())
            .context("Failed to open file")?;
        
        let mmap = unsafe {
            Mmap::map(&file)
                .context("Failed to memory-map file")?
        };

        // Build line offset index
        let line_offsets = Self::build_line_index(&mmap);

        Ok(Self {
            mmap: Arc::new(mmap),
            line_offsets: Arc::new(line_offsets),
            modified_lines: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Build an index of line offsets for fast random access
    fn build_line_index(mmap: &Mmap) -> Vec<usize> {
        let mut offsets = vec![0];
        
        for (i, &byte) in mmap.iter().enumerate() {
            if byte == b'\n' {
                offsets.push(i + 1);
            }
        }
        
        // Ensure we have an offset for EOF if file doesn't end with newline
        if offsets.last() != Some(&mmap.len()) {
            offsets.push(mmap.len());
        }
        
        offsets
    }

    /// Get the total number of lines in the file
    pub fn total_lines(&self) -> usize {
        self.line_offsets.len().saturating_sub(1).max(1)
    }

    /// Get a single line by line number (0-indexed)
    pub fn get_line(&self, line_num: usize) -> Option<String> {
        // Check if line is modified
        if let Ok(modified) = self.modified_lines.read() {
            if let Some(content) = modified.get(&line_num) {
                return Some(content.clone());
            }
        }

        // Get from memory-mapped file
        if line_num >= self.line_offsets.len() - 1 {
            return None;
        }

        let start = self.line_offsets[line_num];
        let end = self.line_offsets[line_num + 1];

        if start >= end {
            return Some(String::new());
        }

        let line_bytes = &self.mmap[start..end];
        
        // Remove trailing newline if present
        let line_bytes = if line_bytes.last() == Some(&b'\n') {
            &line_bytes[..line_bytes.len() - 1]
        } else {
            line_bytes
        };

        // Remove trailing carriage return if present (for Windows line endings)
        let line_bytes = if line_bytes.last() == Some(&b'\r') {
            &line_bytes[..line_bytes.len() - 1]
        } else {
            line_bytes
        };

        String::from_utf8_lossy(line_bytes).to_string().into()
    }

    /// Get a range of lines for viewport rendering
    pub fn get_viewport_lines(&self, start_line: usize, count: usize) -> Vec<String> {
        let total_lines = self.total_lines();
        let end_line = (start_line + count).min(total_lines);
        
        (start_line..end_line)
            .filter_map(|i| self.get_line(i))
            .collect()
    }

    /// Update a line in memory (for editing)
    pub fn update_line(&mut self, line_num: usize, content: String) {
        if let Ok(mut modified) = self.modified_lines.write() {
            modified.insert(line_num, content);
        }
    }

    /// Get all modified lines
    pub fn get_modified_lines(&self) -> HashMap<usize, String> {
        self.modified_lines.read()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Clear all modifications
    #[allow(dead_code)]
    pub fn clear_modifications(&mut self) {
        if let Ok(mut modified) = self.modified_lines.write() {
            modified.clear();
        }
    }

    /// Get raw bytes for a line range (for search operations)
    #[allow(dead_code)]
    pub fn get_line_range_bytes(&self, start_line: usize, end_line: usize) -> &[u8] {
        let total_lines = self.line_offsets.len() - 1;
        if start_line >= total_lines {
            return &[];
        }
        
        let end_line = end_line.min(total_lines);
        let start_offset = self.line_offsets[start_line];
        let end_offset = self.line_offsets[end_line];
        
        &self.mmap[start_offset..end_offset]
    }

    /// Get the byte offset for a given line number
    #[allow(dead_code)]
    pub fn get_line_offset(&self, line_num: usize) -> Option<usize> {
        self.line_offsets.get(line_num).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_file_handler_basic() -> Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "Line 1")?;
        writeln!(temp_file, "Line 2")?;
        writeln!(temp_file, "Line 3")?;
        temp_file.flush()?;

        let handler = FileHandler::new(temp_file.path())?;
        
        assert_eq!(handler.total_lines(), 3);
        assert_eq!(handler.get_line(0), Some("Line 1".to_string()));
        assert_eq!(handler.get_line(1), Some("Line 2".to_string()));
        assert_eq!(handler.get_line(2), Some("Line 3".to_string()));

        Ok(())
    }

    #[test]
    fn test_viewport_lines() -> Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        for i in 1..=100 {
            writeln!(temp_file, "Line {}", i)?;
        }
        temp_file.flush()?;

        let handler = FileHandler::new(temp_file.path())?;
        
        let viewport = handler.get_viewport_lines(0, 10);
        assert_eq!(viewport.len(), 10);
        assert_eq!(viewport[0], "Line 1");
        assert_eq!(viewport[9], "Line 10");

        let viewport = handler.get_viewport_lines(50, 10);
        assert_eq!(viewport.len(), 10);
        assert_eq!(viewport[0], "Line 51");

        Ok(())
    }

    #[test]
    fn test_line_modification() -> Result<()> {
        let mut temp_file = NamedTempFile::new()?;
        writeln!(temp_file, "Original line")?;
        temp_file.flush()?;

        let mut handler = FileHandler::new(temp_file.path())?;
        
        assert_eq!(handler.get_line(0), Some("Original line".to_string()));
        
        handler.update_line(0, "Modified line".to_string());
        assert_eq!(handler.get_line(0), Some("Modified line".to_string()));

        Ok(())
    }
}
