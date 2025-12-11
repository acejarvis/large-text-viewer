use crate::file_handler::FileHandler;
use anyhow::{Context, Result};
use rayon::prelude::*;
use regex::Regex;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Replace all occurrences in the file using parallel processing
pub async fn replace_all(
    handler: &FileHandler,
    file_path: &Path,
    search: &str,
    replace: &str,
) -> Result<(), String> {
    if search.is_empty() {
        return Err("Search pattern cannot be empty".to_string());
    }

    // Determine if size-changing replacement
    let size_changing = search.len() != replace.len();
    
    if size_changing {
        // Use copy-on-write approach for size-changing replacements
        replace_copy_on_write(handler, file_path, search, replace).await
    } else {
        // Use in-place replacement for same-size replacements (safer and faster)
        replace_in_place(handler, file_path, search, replace).await
    }
}

/// Perform in-place replacement for same-size patterns
async fn replace_in_place(
    handler: &FileHandler,
    file_path: &Path,
    search: &str,
    replace: &str,
) -> Result<(), String> {
    // For in-place editing, we create a new file and rename it
    // This is safer than true in-place modification
    replace_copy_on_write(handler, file_path, search, replace).await
}

/// Perform copy-on-write replacement (creates a new file)
async fn replace_copy_on_write(
    handler: &FileHandler,
    file_path: &Path,
    search: &str,
    replace: &str,
) -> Result<(), String> {
    // Create temporary file in the same directory
    let temp_path = file_path.with_extension("tmp");
    
    let total_lines = handler.total_lines();
    let chunk_size = 1000;
    
    // Check if pattern is a valid regex
    let is_regex = Regex::new(search).is_ok();
    
    // Process chunks in parallel
    let chunks: Vec<usize> = (0..total_lines)
        .step_by(chunk_size)
        .collect();
    
    let handler_clone = handler.clone();
    let search_owned = search.to_string();
    let replace_owned = replace.to_string();
    
    let processed_chunks: Vec<Vec<String>> = chunks
        .par_iter()
        .map(|&start| {
            let end = (start + chunk_size).min(total_lines);
            let mut chunk_lines = Vec::new();
            
            for line_num in start..end {
                if let Some(line) = handler_clone.get_line(line_num) {
                    let new_line = if is_regex {
                        if let Ok(regex) = Regex::new(&search_owned) {
                            regex.replace_all(&line, replace_owned.as_str()).to_string()
                        } else {
                            line
                        }
                    } else {
                        line.replace(&search_owned, &replace_owned)
                    };
                    chunk_lines.push(new_line);
                }
            }
            
            chunk_lines
        })
        .collect();
    
    // Write to temporary file
    {
        let file = File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        let mut writer = BufWriter::new(file);
        
        for chunk in processed_chunks {
            for line in chunk {
                writeln!(writer, "{}", line)
                    .map_err(|e| format!("Failed to write line: {}", e))?;
            }
        }
        
        writer.flush()
            .map_err(|e| format!("Failed to flush writer: {}", e))?;
    }
    
    // Replace original file with temporary file
    std::fs::rename(&temp_path, file_path)
        .map_err(|e| format!("Failed to replace original file: {}", e))?;
    
    Ok(())
}

/// Save modified lines back to file
pub async fn save_file(handler: &FileHandler, file_path: &Path) -> Result<(), String> {
    let modified_lines = handler.get_modified_lines();
    
    if modified_lines.is_empty() {
        return Ok(());
    }
    
    // Create temporary file
    let temp_path = file_path.with_extension("tmp");
    
    let total_lines = handler.total_lines();
    
    {
        let file = File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        let mut writer = BufWriter::new(file);
        
        for line_num in 0..total_lines {
            let line = if let Some(modified) = modified_lines.get(&line_num) {
                modified.clone()
            } else {
                handler.get_line(line_num).unwrap_or_default()
            };
            
            writeln!(writer, "{}", line)
                .map_err(|e| format!("Failed to write line: {}", e))?;
        }
        
        writer.flush()
            .map_err(|e| format!("Failed to flush writer: {}", e))?;
    }
    
    // Replace original file
    std::fs::rename(&temp_path, file_path)
        .map_err(|e| format!("Failed to replace original file: {}", e))?;
    
    Ok(())
}

/// Create a backup of the file before editing
#[allow(dead_code)]
pub fn create_backup(file_path: &Path) -> Result<PathBuf> {
    let backup_path = file_path.with_extension("bak");
    std::fs::copy(file_path, &backup_path)
        .context("Failed to create backup")?;
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_replace_all_literal() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Hello world").unwrap();
        writeln!(temp_file, "Hello again").unwrap();
        writeln!(temp_file, "Goodbye").unwrap();
        temp_file.flush().unwrap();

        let path = temp_file.path().to_path_buf();
        let handler = FileHandler::new(&path).unwrap();
        
        replace_all(&handler, &path, "Hello", "Hi").await.unwrap();
        
        // Read the file back
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
        
        assert_eq!(lines[0], "Hi world");
        assert_eq!(lines[1], "Hi again");
        assert_eq!(lines[2], "Goodbye");
    }

    #[tokio::test]
    async fn test_replace_all_regex() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "test123").unwrap();
        writeln!(temp_file, "test456").unwrap();
        writeln!(temp_file, "nodigits").unwrap();
        temp_file.flush().unwrap();

        let path = temp_file.path().to_path_buf();
        let handler = FileHandler::new(&path).unwrap();
        
        replace_all(&handler, &path, r"test\d+", "replaced").await.unwrap();
        
        // Read the file back
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
        
        assert_eq!(lines[0], "replaced");
        assert_eq!(lines[1], "replaced");
        assert_eq!(lines[2], "nodigits");
    }

    #[tokio::test]
    async fn test_save_file() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Line 1").unwrap();
        writeln!(temp_file, "Line 2").unwrap();
        temp_file.flush().unwrap();

        let path = temp_file.path().to_path_buf();
        let mut handler = FileHandler::new(&path).unwrap();
        
        handler.update_line(0, "Modified Line 1".to_string());
        
        save_file(&handler, &path).await.unwrap();
        
        // Read the file back
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().map(|l| l.unwrap()).collect();
        
        assert_eq!(lines[0], "Modified Line 1");
        assert_eq!(lines[1], "Line 2");
    }
}
