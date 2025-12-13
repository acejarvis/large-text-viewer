use regex::Regex;
use crate::file_reader::FileReader;
use std::sync::{Arc, mpsc::SyncSender, atomic::{AtomicBool, Ordering}};
use std::thread;

pub struct SearchEngine {
    query: String,
    use_regex: bool,
    regex: Option<Regex>,
    results: Vec<SearchResult>,
    total_results: usize,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub byte_offset: usize,
    pub match_len: usize,
}

pub struct ChunkSearchResult {
    pub matches: Vec<SearchResult>,
}

pub enum SearchMessage {
    ChunkResult(ChunkSearchResult),
    CountResult(usize),
    Done,
    Error(String),
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            use_regex: false,
            regex: None,
            results: Vec::new(),
            total_results: 0,
        }
    }

    pub fn set_query(&mut self, query: String, use_regex: bool) {
        self.query = query;
        self.use_regex = use_regex;
        
        if use_regex {
            self.regex = Regex::new(&self.query).ok();
        } else {
            let pattern = format!("(?i){}", regex::escape(&self.query));
            self.regex = Regex::new(&pattern).ok();
        }
        
        self.results.clear();
    }

    pub fn count_matches(
        &self,
        reader: Arc<FileReader>,
        tx: SyncSender<SearchMessage>,
        cancel_token: Arc<AtomicBool>,
    ) {
        let file_len = reader.len();
        if file_len == 0 || self.query.is_empty() {
            let _ = tx.send(SearchMessage::CountResult(0));
            let _ = tx.send(SearchMessage::Done);
            return;
        }

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);

        let chunk_size = (file_len + num_threads - 1) / num_threads;
        let query_len = self.query.len();
        let overlap = query_len.saturating_sub(1).max(1000);

        let regex = self.regex.clone();

        thread::spawn(move || {
            let mut handles = vec![];

            for i in 0..num_threads {
                let thread_start = i * chunk_size;
                if thread_start >= file_len {
                    break;
                }
                let thread_end = (thread_start + chunk_size).min(file_len);
                
                let reader_clone = reader.clone();
                let tx_clone = tx.clone();
                let regex_clone = regex.clone();
                let cancel_token_clone = cancel_token.clone();

                let handle = thread::spawn(move || {
                    if let Some(regex) = regex_clone {
                        let mut pos = thread_start;
                        // Process in smaller batches to avoid high memory usage
                        const BATCH_SIZE: usize = 4 * 1024 * 1024; // 4MB
                        let mut local_count = 0;

                        while pos < thread_end {
                            if cancel_token_clone.load(Ordering::Relaxed) {
                                return;
                            }

                            let batch_end = (pos + BATCH_SIZE).min(thread_end);
                            // Add overlap to catch matches crossing batch boundaries
                            let read_end = (batch_end + overlap).min(file_len);
                            
                            let chunk_bytes = reader_clone.get_bytes(pos, read_end);
                            let chunk_text = match std::str::from_utf8(chunk_bytes) {
                                Ok(t) => t.to_string(),
                                Err(_) => {
                                    let (cow, _, _) = reader_clone.encoding().decode(chunk_bytes);
                                    cow.into_owned()
                                }
                            };

                            for mat in regex.find_iter(&chunk_text) {
                                if cancel_token_clone.load(Ordering::Relaxed) {
                                    return;
                                }
                                let match_start = mat.start();
                                let absolute_start = pos + match_start;
                                
                                // Only accept matches starting in [pos, batch_end)
                                if absolute_start >= batch_end {
                                    continue;
                                }
                                
                                local_count += 1;
                            }
                            
                            pos = batch_end;
                        }
                        let _ = tx_clone.send(SearchMessage::CountResult(local_count));
                    } else {
                         let _ = tx_clone.send(SearchMessage::Error("Invalid regex".to_string()));
                    }
                });
                handles.push(handle);
            }

            for h in handles {
                let _ = h.join();
            }
            if !cancel_token.load(Ordering::Relaxed) {
                let _ = tx.send(SearchMessage::Done);
            }
        });
    }

    pub fn fetch_matches(
        &self,
        reader: Arc<FileReader>,
        tx: SyncSender<SearchMessage>,
        start_offset: usize,
        max_results: usize,
        cancel_token: Arc<AtomicBool>,
    ) {
        let file_len = reader.len();
        if file_len == 0 || self.query.is_empty() {
            let _ = tx.send(SearchMessage::Done);
            return;
        }

        let regex = self.regex.clone();
        let query_len = self.query.len();
        let overlap = query_len.saturating_sub(1).max(1000);

        thread::spawn(move || {
            if let Some(regex) = regex {
                const CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10 MB chunks
                let mut chunk_start = start_offset;
                let mut results_found = 0;
                
                while chunk_start < file_len && results_found < max_results {
                    if cancel_token.load(Ordering::Relaxed) {
                        return;
                    }

                    let chunk_end = (chunk_start + CHUNK_SIZE).min(file_len);
                    let chunk_bytes = reader.get_bytes(chunk_start, chunk_end);
                    
                    let chunk_text = match std::str::from_utf8(chunk_bytes) {
                        Ok(t) => t.to_string(),
                        Err(_) => {
                            let (cow, _, _) = reader.encoding().decode(chunk_bytes);
                            cow.into_owned()
                        }
                    };
                    
                    let mut local_matches = Vec::new();
                    
                    // Define the valid range for starting positions in this chunk
                    // We want to process matches that start in [chunk_start, chunk_end - overlap)
                    // Unless we are at the end of the file, then [chunk_start, chunk_end)
                    let valid_end = if chunk_end >= file_len {
                        file_len
                    } else {
                        chunk_end - overlap
                    };

                    for mat in regex.find_iter(&chunk_text) {
                        if cancel_token.load(Ordering::Relaxed) {
                            return;
                        }
                        if results_found >= max_results {
                            break;
                        }
                        
                        let match_start = mat.start();
                        let absolute_start = chunk_start + match_start;
                        
                        // Skip matches that start beyond our valid range for this chunk
                        // They will be picked up by the next chunk which starts at `valid_end`
                        if absolute_start >= valid_end {
                            continue;
                        }
                        
                        local_matches.push(SearchResult {
                            byte_offset: absolute_start,
                            match_len: mat.end() - mat.start(),
                        });
                        results_found += 1;
                    }
                    
                    if !local_matches.is_empty() {

                        if tx.send(SearchMessage::ChunkResult(ChunkSearchResult {
                            matches: local_matches,
                        })).is_err() {
                            return;
                        }
                    }
                    
                    // Move to next chunk with overlap
                    if chunk_end >= file_len {
                        break;
                    }
                    
                    chunk_start = chunk_end - overlap;
                }
                if !cancel_token.load(Ordering::Relaxed) {
                    let _ = tx.send(SearchMessage::Done);
                }
            } else {
                 let _ = tx.send(SearchMessage::Error("Invalid regex".to_string()));
            }
        });
    }

    pub fn search_parallel(
        &self,
        reader: Arc<FileReader>,
        tx: SyncSender<SearchMessage>,
        _max_results: usize,
    ) {
        let file_len = reader.len();
        if file_len == 0 || self.query.is_empty() {
            let _ = tx.send(SearchMessage::Done);
            return;
        }

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .max(1);

        let chunk_size = (file_len + num_threads - 1) / num_threads;
        let query_len = self.query.len();
        let overlap = query_len.saturating_sub(1).max(1000);

        let regex = self.regex.clone();

        thread::spawn(move || {
            let mut handles = vec![];

            for i in 0..num_threads {
                let thread_start = i * chunk_size;
                if thread_start >= file_len {
                    break;
                }
                let thread_end = (thread_start + chunk_size).min(file_len);
                
                let reader_clone = reader.clone();
                let tx_clone = tx.clone();
                let regex_clone = regex.clone();

                let handle = thread::spawn(move || {
                    if let Some(regex) = regex_clone {
                        let mut pos = thread_start;
                        // Process in smaller batches to avoid high memory usage
                        const BATCH_SIZE: usize = 4 * 1024 * 1024; // 4MB

                        while pos < thread_end {
                            let batch_end = (pos + BATCH_SIZE).min(thread_end);
                            // Add overlap to catch matches crossing batch boundaries
                            let read_end = (batch_end + overlap).min(file_len);
                            
                            let chunk_bytes = reader_clone.get_bytes(pos, read_end);
                            let chunk_text = match std::str::from_utf8(chunk_bytes) {
                                Ok(t) => t.to_string(),
                                Err(_) => {
                                    let (cow, _, _) = reader_clone.encoding().decode(chunk_bytes);
                                    cow.into_owned()
                                }
                            };

                            let mut local_matches = Vec::new();
                            
                            for mat in regex.find_iter(&chunk_text) {
                                let match_start = mat.start();
                                let absolute_start = pos + match_start;
                                
                                // Only accept matches starting in [pos, batch_end)
                                // Matches starting >= batch_end will be handled by the next batch (thanks to overlap)
                                if absolute_start >= batch_end {
                                    continue;
                                }
                                
                                local_matches.push(SearchResult {
                                    byte_offset: absolute_start,
                                    match_len: mat.end() - mat.start(),
                                });
                            }
                            
                            if !local_matches.is_empty() {
                                // This will block if the channel is full, providing backpressure
                                if tx_clone.send(SearchMessage::ChunkResult(ChunkSearchResult {
                                    matches: local_matches,
                                })).is_err() {
                                    // Receiver dropped, stop searching
                                    return;
                                }
                            }

                            pos = batch_end;
                        }
                    } else {
                         let _ = tx_clone.send(SearchMessage::Error("Invalid regex".to_string()));
                    }
                });
                handles.push(handle);
            }

            for h in handles {
                let _ = h.join();
            }
            let _ = tx.send(SearchMessage::Done);
        });
    }

    #[allow(dead_code)]
    pub fn search(&mut self, reader: &FileReader, max_results: usize) -> Result<(), String> {
        self.results.clear();

        if self.query.is_empty() {
            return Ok(());
        }

        self.total_results = 0;

        // Use chunked search to avoid loading entire file into memory
        self.search_chunked(reader, max_results)
    }

    #[allow(dead_code)]
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
            self.search_chunk_regex(&chunk_text, chunk_start, &mut line_number, max_results)?;
            
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

    #[allow(dead_code)]
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
                
                // Count every match, even if we stop storing due to max_results
                self.total_results += 1;

                if self.results.len() < max_results {
                    self.results.push(SearchResult {
                        byte_offset: chunk_offset + mat.start(),
                        match_len: mat.end() - mat.start(),
                    });
                }
            }
            
            // Update line number for remaining chunk
            *line_number = current_line + chunk_text[last_pos..].lines().count().saturating_sub(1);
            
            Ok(())
        } else {
            Err("Invalid regex pattern".to_string())
        }
    }

    #[allow(dead_code)]
    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    #[allow(dead_code)]
    pub fn total_results(&self) -> usize {
        self.total_results
    }

    #[allow(dead_code)]
    pub fn has_results(&self) -> bool {
        !self.results.is_empty()
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.results.clear();
        self.regex = None;
        self.total_results = 0;
    }
}
