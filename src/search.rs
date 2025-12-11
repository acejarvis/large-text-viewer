use crate::file_handler::FileHandler;
use rayon::prelude::*;
use regex::Regex;

/// Perform parallel search across the file
pub async fn search_file(handler: &FileHandler, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }

    let total_lines = handler.total_lines();
    let chunk_size = 1000; // Process 1000 lines per chunk
    
    // Try to compile as regex first, fall back to literal search
    let is_regex = Regex::new(query).is_ok();
    
    if is_regex {
        search_regex(handler, query, total_lines, chunk_size).await
    } else {
        search_literal(handler, query, total_lines, chunk_size).await
    }
}

/// Perform regex search using parallel processing
async fn search_regex(
    handler: &FileHandler,
    pattern: &str,
    total_lines: usize,
    chunk_size: usize,
) -> Vec<usize> {
    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let chunks: Vec<usize> = (0..total_lines)
        .step_by(chunk_size)
        .collect();

    let handler_clone = handler.clone();
    
    // Use rayon for parallel search across chunks
    let results: Vec<Vec<usize>> = chunks
        .par_iter()
        .map(|&start| {
            let end = (start + chunk_size).min(total_lines);
            let mut matches = Vec::new();
            
            for line_num in start..end {
                if let Some(line) = handler_clone.get_line(line_num) {
                    if regex.is_match(&line) {
                        matches.push(line_num);
                    }
                }
            }
            
            matches
        })
        .collect();

    // Flatten results
    let mut all_matches: Vec<usize> = results.into_iter().flatten().collect();
    all_matches.sort_unstable();
    all_matches
}

/// Perform literal string search using parallel processing
async fn search_literal(
    handler: &FileHandler,
    query: &str,
    total_lines: usize,
    chunk_size: usize,
) -> Vec<usize> {
    let chunks: Vec<usize> = (0..total_lines)
        .step_by(chunk_size)
        .collect();

    let handler_clone = handler.clone();
    let query_owned = query.to_string();
    
    // Use rayon for parallel search across chunks
    let results: Vec<Vec<usize>> = chunks
        .par_iter()
        .map(|&start| {
            let end = (start + chunk_size).min(total_lines);
            let mut matches = Vec::new();
            
            for line_num in start..end {
                if let Some(line) = handler_clone.get_line(line_num) {
                    if line.contains(&query_owned) {
                        matches.push(line_num);
                    }
                }
            }
            
            matches
        })
        .collect();

    // Flatten results
    let mut all_matches: Vec<usize> = results.into_iter().flatten().collect();
    all_matches.sort_unstable();
    all_matches
}

/// Advanced search with context lines
#[allow(dead_code)]
pub async fn search_with_context(
    handler: &FileHandler,
    query: &str,
    context_lines: usize,
) -> Vec<(usize, Vec<String>)> {
    let matches = search_file(handler, query).await;
    let total_lines = handler.total_lines();
    
    matches
        .into_iter()
        .map(|line_num| {
            let start = line_num.saturating_sub(context_lines);
            let end = (line_num + context_lines + 1).min(total_lines);
            
            let context: Vec<String> = (start..end)
                .filter_map(|i| handler.get_line(i))
                .collect();
            
            (line_num, context)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_literal_search() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "Hello world").unwrap();
        writeln!(temp_file, "Goodbye world").unwrap();
        writeln!(temp_file, "Hello again").unwrap();
        temp_file.flush().unwrap();

        let handler = FileHandler::new(temp_file.path()).unwrap();
        let results = search_file(&handler, "Hello").await;
        
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], 0);
        assert_eq!(results[1], 2);
    }

    #[tokio::test]
    async fn test_regex_search() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "test123").unwrap();
        writeln!(temp_file, "test456").unwrap();
        writeln!(temp_file, "nodigits").unwrap();
        temp_file.flush().unwrap();

        let handler = FileHandler::new(temp_file.path()).unwrap();
        let results = search_file(&handler, r"test\d+").await;
        
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], 0);
        assert_eq!(results[1], 1);
    }

    #[tokio::test]
    async fn test_search_with_context() {
        let mut temp_file = NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(temp_file, "Line {}", i).unwrap();
        }
        temp_file.flush().unwrap();

        let handler = FileHandler::new(temp_file.path()).unwrap();
        let results = search_with_context(&handler, "Line 5", 1).await;
        
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 4); // 0-indexed
        assert_eq!(results[0].1.len(), 3); // Context: lines 4, 5, 6
    }
}
