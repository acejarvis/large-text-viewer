use regex::Regex;
use crate::file_reader::FileReader;

pub struct SearchEngine {
    query: String,
    use_regex: bool,
    regex: Option<Regex>,
    results: Vec<SearchResult>,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub byte_offset: usize,
    pub line_number: usize,
    pub match_text: String,
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            use_regex: false,
            regex: None,
            results: Vec::new(),
        }
    }

    pub fn set_query(&mut self, query: String, use_regex: bool) {
        self.query = query;
        self.use_regex = use_regex;
        
        if use_regex {
            self.regex = Regex::new(&self.query).ok();
        } else {
            self.regex = None;
        }
        
        self.results.clear();
    }

    pub fn search(&mut self, reader: &FileReader, max_results: usize) -> Result<(), String> {
        self.results.clear();

        if self.query.is_empty() {
            return Ok(());
        }

        // Use chunked search to avoid loading entire file into memory
        self.search_chunked(reader, max_results)
    }

    fn search_chunked(&mut self, reader: &FileReader, max_results: usize) -> Result<(), String> {
        const CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10 MB chunks
        let file_len = reader.len();
        let query_len = self.query.len();
        
        // Overlap to catch matches across chunk boundaries
        let overlap = query_len.saturating_sub(1).max(1000);
        
        let mut chunk_start = 0;
        let mut line_number = 0;
        
        while chunk_start < file_len && self.results.len() < max_results {
            let chunk_end = (chunk_start + CHUNK_SIZE).min(file_len);
            let chunk_bytes = reader.get_bytes(chunk_start, chunk_end);
            
            // Decode chunk
            let chunk_text = match std::str::from_utf8(chunk_bytes) {
                Ok(t) => t.to_string(),
                Err(_) => {
                    let (cow, _, _) = reader.encoding().decode(chunk_bytes);
                    cow.into_owned()
                }
            };
            
            // Search in this chunk
            if self.use_regex {
                self.search_chunk_regex(&chunk_text, chunk_start, &mut line_number, max_results)?;
            } else {
                self.search_chunk_simple(&chunk_text, chunk_start, &mut line_number, max_results);
            }
            
            // Move to next chunk with overlap
            if chunk_end >= file_len {
                break;
            }
            
            chunk_start = chunk_end - overlap;
            
            // Recalculate line number for overlap region to avoid double counting
            let overlap_bytes = reader.get_bytes(chunk_start, chunk_end);
            let overlap_text = match std::str::from_utf8(overlap_bytes) {
                Ok(t) => t.to_string(),
                Err(_) => {
                    let (cow, _, _) = reader.encoding().decode(overlap_bytes);
                    cow.into_owned()
                }
            };
            line_number -= overlap_text.lines().count().saturating_sub(1);
        }

        Ok(())
    }

    fn search_chunk_simple(
        &mut self,
        chunk_text: &str,
        chunk_offset: usize,
        line_number: &mut usize,
        max_results: usize,
    ) {
        let query_lower = self.query.to_lowercase();
        let chunk_lower = chunk_text.to_lowercase();
        let mut pos = 0;
        let mut current_line = *line_number;
        let mut last_newline = 0;
        
        while let Some(match_pos) = chunk_lower[pos..].find(&query_lower) {
            let absolute_pos = pos + match_pos;
            
            // Count newlines up to this position (byte-based iteration)
            for ch in chunk_text[last_newline..absolute_pos].chars() {
                if ch == '\n' {
                    current_line += 1;
                }
            }
            last_newline = absolute_pos;
            
            // Extract actual match text (preserve case)
            let match_text = chunk_text[absolute_pos..absolute_pos.min(absolute_pos + self.query.len())].to_string();
            
            self.results.push(SearchResult {
                byte_offset: chunk_offset + absolute_pos,
                line_number: current_line,
                match_text,
            });
            
            if self.results.len() >= max_results {
                break;
            }
            
            pos = absolute_pos + 1;
        }
        
        // Update line number for remaining chunk
        *line_number = current_line + chunk_text[last_newline..].lines().count().saturating_sub(1);
    }

    fn search_chunk_regex(
        &mut self,
        chunk_text: &str,
        chunk_offset: usize,
        line_number: &mut usize,
        max_results: usize,
    ) -> Result<(), String> {
        if let Some(ref regex) = self.regex {
            let mut current_line = *line_number;
            let mut last_pos = 0;
            
            for mat in regex.find_iter(chunk_text) {
                if self.results.len() >= max_results {
                    break;
                }
                
                // Count newlines up to this match
                for ch in chunk_text[last_pos..mat.start()].chars() {
                    if ch == '\n' {
                        current_line += 1;
                    }
                }
                last_pos = mat.start();
                
                self.results.push(SearchResult {
                    byte_offset: chunk_offset + mat.start(),
                    line_number: current_line,
                    match_text: mat.as_str().to_string(),
                });
            }
            
            // Update line number for remaining chunk
            *line_number = current_line + chunk_text[last_pos..].lines().count().saturating_sub(1);
            
            Ok(())
        } else {
            Err("Invalid regex pattern".to_string())
        }
    }

    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    pub fn has_results(&self) -> bool {
        !self.results.is_empty()
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.results.clear();
        self.regex = None;
    }
}
