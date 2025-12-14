use crate::file_reader::FileReader;
use regex::Regex;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc,
};
use std::thread;

pub struct SearchEngine {
    query: String,
    use_regex: bool,
    case_sensitive: bool,
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchType {
    Count,
    Fetch,
}

pub enum SearchMessage {
    ChunkResult(ChunkSearchResult),
    CountResult(usize),
    Done(SearchType),
    Error(String),
}

impl SearchEngine {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            use_regex: false,
            case_sensitive: false,
            regex: None,
            results: Vec::new(),
            total_results: 0,
        }
    }

    pub fn set_query(&mut self, query: String, use_regex: bool, case_sensitive: bool) {
        self.query = query;
        self.use_regex = use_regex;
        self.case_sensitive = case_sensitive;

        let pattern = if use_regex {
            if !case_sensitive {
                format!("(?i){}", self.query)
            } else {
                self.query.clone()
            }
        } else if !case_sensitive {
            format!("(?i){}", regex::escape(&self.query))
        } else {
            regex::escape(&self.query)
        };

        self.regex = Regex::new(&pattern).ok();

        self.results.clear();
    }

    pub fn find_in_text(&self, text: &str) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();
        if self.query.is_empty() {
            return matches;
        }

        if let Some(re) = &self.regex {
            for m in re.find_iter(text) {
                matches.push((m.start(), m.end()));
            }
        }
        matches
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
            let _ = tx.send(SearchMessage::Done(SearchType::Count));
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
                let _ = tx.send(SearchMessage::Done(SearchType::Count));
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
            let _ = tx.send(SearchMessage::Done(SearchType::Fetch));
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
                        if tx
                            .send(SearchMessage::ChunkResult(ChunkSearchResult {
                                matches: local_matches,
                            }))
                            .is_err()
                        {
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
                    let _ = tx.send(SearchMessage::Done(SearchType::Fetch));
                }
            } else {
                let _ = tx.send(SearchMessage::Error("Invalid regex".to_string()));
            }
        });
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.results.clear();
        self.regex = None;
        self.total_results = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use crate::file_reader::detect_encoding;
    use std::sync::mpsc;

    #[test]
    fn test_find_in_text() {
        let mut engine = SearchEngine::new();
        engine.set_query("test".to_string(), false, false);

        let text = "This is a test string. Another test.";
        let matches = engine.find_in_text(text);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (10, 14));
        assert_eq!(matches[1], (31, 35));
    }

    #[test]
    fn test_find_in_text_regex() {
        let mut engine = SearchEngine::new();
        engine.set_query(r"\d+".to_string(), true, false);

        let text = "There are 123 apples and 456 oranges.";
        let matches = engine.find_in_text(text);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (10, 13)); // "123"
        assert_eq!(matches[1], (25, 28)); // "456"
    }

    #[test]
    fn test_count_matches() -> anyhow::Result<()> {
        let mut file = NamedTempFile::new()?;
        write!(file, "test\ntest\ntest")?;
        let path = file.path().to_path_buf();

        let reader = Arc::new(FileReader::new(path, detect_encoding(b""))?);
        let mut engine = SearchEngine::new();
        engine.set_query("test".to_string(), false, false);

        let (tx, rx) = mpsc::sync_channel(10);
        let cancel_token = Arc::new(AtomicBool::new(false));

        engine.count_matches(reader, tx, cancel_token);

        let mut count = 0;
        loop {
            match rx.recv() {
                Ok(SearchMessage::CountResult(c)) => count += c,
                Ok(SearchMessage::Done(SearchType::Count)) => break,
                Ok(SearchMessage::Error(e)) => panic!("Error: {}", e),
                Ok(_) => continue,
                Err(_) => break,
            }
        }

        assert_eq!(count, 3);
        Ok(())
    }
}
