use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use notify::{Watcher, RecursiveMode, Result as NotifyResult};
use encoding_rs::Encoding;

use crate::file_reader::{FileReader, detect_encoding, available_encodings};
use crate::line_indexer::LineIndexer;
use crate::search_engine::{SearchEngine, SearchResult, SearchMessage, SearchType};
use crate::replacer::{Replacer, ReplaceMessage};

pub struct TextViewerApp {
    file_reader: Option<Arc<FileReader>>,
    line_indexer: LineIndexer,
    search_engine: SearchEngine,
    
    // UI State
    scroll_line: usize,
    visible_lines: usize,
    font_size: f32,
    wrap_mode: bool,
    dark_mode: bool,
    show_line_numbers: bool,
    
    // Search UI
    search_query: String,
    replace_query: String,
    show_replace: bool,
    use_regex: bool,
    search_results: Vec<SearchResult>,
    current_result_index: usize, // Global index (0 to total_results - 1)
    total_search_results: usize,
    search_page_start_index: usize, // Global index of the first result in search_results
    page_offsets: Vec<usize>, // Map of page_index -> start_byte_offset
    search_error: Option<String>,
    search_in_progress: bool,
    search_find_all: bool,
    search_message_rx: Option<Receiver<SearchMessage>>,
    search_cancellation_token: Option<Arc<AtomicBool>>,
    search_count_done: bool,
    search_fetch_done: bool,
    
    // Replace UI
    replace_in_progress: bool,
    replace_message_rx: Option<Receiver<ReplaceMessage>>,
    replace_cancellation_token: Option<Arc<AtomicBool>>,
    replace_progress: Option<f32>,
    replace_status_message: Option<String>,
    
    // Go to line
    goto_line_input: String,
    
    // File info
    show_file_info: bool,
    
    // Tail mode
    tail_mode: bool,
    watcher: Option<Box<dyn Watcher>>,
    file_change_rx: Option<Receiver<()>>,
    
    // Status messages
    status_message: String,
    
    // Encoding
    selected_encoding: &'static Encoding,
    show_encoding_selector: bool,
    
    // Programmatic scroll control
    scroll_to_row: Option<usize>,
}

impl Default for TextViewerApp {
    fn default() -> Self {
        Self {
            file_reader: None,
            line_indexer: LineIndexer::new(),
            search_engine: SearchEngine::new(),
            scroll_line: 0,
            visible_lines: 50,
            font_size: 14.0,
            wrap_mode: false,
            dark_mode: true,
            show_line_numbers: true,
            search_query: String::new(),
            replace_query: String::new(),
            show_replace: false,
            use_regex: false,
            search_results: Vec::new(),
            current_result_index: 0,
            total_search_results: 0,
            search_page_start_index: 0,
            page_offsets: Vec::new(),
            search_error: None,
            search_in_progress: false,
            search_find_all: true,
            search_message_rx: None,
            search_cancellation_token: None,
            search_count_done: false,
            search_fetch_done: false,
            replace_in_progress: false,
            replace_message_rx: None,
            replace_cancellation_token: None,
            replace_progress: None,
            replace_status_message: None,
            goto_line_input: String::new(),
            show_file_info: false,
            tail_mode: false,
            watcher: None,
            file_change_rx: None,
            status_message: String::new(),
            selected_encoding: encoding_rs::UTF_8,
            show_encoding_selector: false,
            scroll_to_row: None,
        }
    }
}

impl TextViewerApp {
    fn open_file(&mut self, path: PathBuf) {
        match FileReader::new(path.clone(), self.selected_encoding) {
            Ok(reader) => {
                self.file_reader = Some(Arc::new(reader));
                self.line_indexer.index_file(self.file_reader.as_ref().unwrap());
                self.scroll_line = 0;
                self.scroll_to_row = Some(0); // Reset scroll to top for new file
                self.status_message = format!("Opened: {}", path.display());
                self.search_engine.clear();
                self.search_results.clear();
                self.total_search_results = 0;
                self.search_page_start_index = 0;
                self.page_offsets.clear();
                self.current_result_index = 0;
                
                // Setup file watcher if tail mode is enabled
                if self.tail_mode {
                    self.setup_file_watcher();
                }
            }
            Err(e) => {
                self.status_message = format!("Error opening file: {}", e);
            }
        }
    }

    fn setup_file_watcher(&mut self) {
        if let Some(ref reader) = self.file_reader {
            let (tx, rx) = channel();
            let path = reader.path().clone();
            
            match notify::recommended_watcher(move |res: NotifyResult<notify::Event>| {
                if let Ok(_event) = res {
                    let _ = tx.send(());
                }
            }) {
                Ok(mut watcher) => {
                    if watcher.watch(&path, RecursiveMode::NonRecursive).is_ok() {
                        self.watcher = Some(Box::new(watcher));
                        self.file_change_rx = Some(rx);
                    }
                }
                Err(_) => {}
            }
        }
    }

    fn check_file_changes(&mut self) {
        if let Some(ref rx) = self.file_change_rx {
            if rx.try_recv().is_ok() {
                // File changed, reload
                if let Some(ref reader) = self.file_reader {
                    let path = reader.path().clone();
                    let encoding = reader.encoding();
                    self.selected_encoding = encoding;
                    self.open_file(path);
                    
                    // Scroll to bottom in tail mode
                    if self.tail_mode {
                        let total_lines = self.line_indexer.total_lines();
                        let target_line = total_lines.saturating_sub(self.visible_lines);
                        self.scroll_line = target_line;
                        self.scroll_to_row = Some(target_line);
                    }
                }
            }
        }
    }

    fn perform_search(&mut self, find_all: bool) {
        self.search_error = None;
        self.search_results.clear();
        self.current_result_index = 0;
        self.total_search_results = 0;
        self.search_page_start_index = 0;
        self.page_offsets.clear();
        self.search_engine.clear();

        if self.search_in_progress {
            self.status_message = "Search already running...".to_string();
            return;
        }

        let Some(ref reader) = self.file_reader else {
            self.status_message = "Open a file before searching".to_string();
            return;
        };

        if self.search_query.is_empty() {
            self.status_message = "Enter a search query first".to_string();
            return;
        }

        self.search_engine.set_query(self.search_query.clone(), self.use_regex);

        let reader = reader.clone();
        // Use a bounded channel to provide backpressure to search threads
        // This prevents memory explosion if the UI thread can't keep up with results
        let (tx, rx) = std::sync::mpsc::sync_channel(10_000);

        self.search_message_rx = Some(rx);
        self.search_in_progress = true;
        self.search_find_all = find_all;
        self.search_count_done = false;
        self.search_fetch_done = false;
        
        let cancel_token = Arc::new(AtomicBool::new(false));
        self.search_cancellation_token = Some(cancel_token.clone());
        
        self.status_message = if find_all {
            "Searching all matches...".to_string()
        } else {
            "Searching first match...".to_string()
        };

        if find_all {
            // Start two tasks:
            // 1. Count all matches (parallel)
            // 2. Fetch first page of matches (sequential/chunked)
            
            let tx_count = tx.clone();
            let reader_count = reader.clone();
            let query = self.search_query.clone();
            let use_regex = self.use_regex;
            let cancel_token_count = cancel_token.clone();
            
            std::thread::spawn(move || {
                // Task 1: Count
                let mut engine = SearchEngine::new();
                engine.set_query(query, use_regex);
                engine.count_matches(reader_count, tx_count, cancel_token_count);
            });
            
            let tx_fetch = tx.clone();
            let reader_fetch = reader.clone();
            let query_fetch = self.search_query.clone();
            let cancel_token_fetch = cancel_token.clone();
            
            std::thread::spawn(move || {
                // Task 2: Fetch first page
                let mut engine = SearchEngine::new();
                engine.set_query(query_fetch, use_regex);
                engine.fetch_matches(reader_fetch, tx_fetch, 0, 1000, cancel_token_fetch);
            });
            
        } else {
            // Find first match only
             let tx_fetch = tx.clone();
             let reader_fetch = reader.clone();
             let query = self.search_query.clone();
             let use_regex = self.use_regex;
             let cancel_token_fetch = cancel_token.clone();
             
             std::thread::spawn(move || {
                let mut engine = SearchEngine::new();
                engine.set_query(query, use_regex);
                engine.fetch_matches(reader_fetch, tx_fetch, 0, 1, cancel_token_fetch);
            });
        }
    }

    fn poll_search_results(&mut self) {
        if !self.search_in_progress {
            return;
        }

        if let Some(ref rx) = self.search_message_rx {
            let mut new_results_added = false;
            // Process all available messages
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    SearchMessage::CountResult(count) => {
                        self.total_search_results += count;
                        if self.search_find_all {
                            self.status_message = format!("Found {} matches...", self.total_search_results);
                        }
                    }
                    SearchMessage::ChunkResult(chunk_result) => {
                        // Add results
                        self.search_results.extend(chunk_result.matches);
                        new_results_added = true;
                        
                        // If we found results and haven't scrolled yet, scroll to the first one
                        if !self.search_results.is_empty() && self.scroll_to_row.is_none() && self.current_result_index == 0 {
                             // We need to sort at least once to find the true first result
                             // But doing it here might be expensive if we do it often.
                             // For the very first result, we can just check if we have any.
                             // However, to be correct, we should probably wait or do a partial check.
                             // For now, let's defer the sort to outside the loop.
                        }
                    }
                    SearchMessage::Done(search_type) => {
                        match search_type {
                            SearchType::Count => self.search_count_done = true,
                            SearchType::Fetch => self.search_fetch_done = true,
                        }
                        
                        if self.search_find_all && self.search_count_done {
                             if self.search_results.len() == self.total_search_results {
                                 if let Some(token) = &self.search_cancellation_token {
                                     token.store(true, Ordering::Relaxed);
                                 }
                             }
                        }
                    }
                    SearchMessage::Error(e) => {
                        self.search_in_progress = false;
                        self.search_message_rx = None;
                        self.search_error = Some(e.clone());
                        self.status_message = format!("Search failed: {}", e);
                        return; // Stop processing messages
                    }
                }
            }
            
            // Check if channel is disconnected
            if let Err(std::sync::mpsc::TryRecvError::Disconnected) = rx.try_recv() {
                self.search_in_progress = false;
                self.search_message_rx = None;
                
                // Final sort to ensure everything is in order
                self.search_results.sort_by_key(|r| r.byte_offset);
                
                // If we are in "Find All" mode, total_results should be at least search_results.len()
                // But count task might be slower or faster.
                // If count task finished, total_results is correct.
                // If fetch task finished, search_results is populated.
                
                // If we are not finding all, total_results might be 0 (since we didn't run count task).
                if !self.search_find_all {
                    self.total_search_results = self.search_results.len();
                } else {
                    // Ensure total is at least what we have
                    self.total_search_results = self.total_search_results.max(self.search_results.len());
                }

                let total = self.total_search_results;
                if total > 0 {
                    if self.search_find_all {
                        self.status_message = format!("Found {} matches", total);
                    } else {
                        self.status_message = "Showing first match. Run Find All to see every result.".to_string();
                    }
                    
                    // Ensure we scroll to the first result if we haven't yet
                    if self.scroll_to_row.is_none() && !self.search_results.is_empty() {
                         let target_line = self.line_indexer.find_line_at_offset(self.search_results[0].byte_offset);
                         self.scroll_line = target_line;
                         self.scroll_to_row = Some(target_line);
                    }
                } else {
                    self.status_message = "No matches found".to_string();
                }
            }
            
            if new_results_added {
                // Sort results by byte offset to keep them in order
                // Only sort once per frame after processing all available chunks
                self.search_results.sort_by_key(|r| r.byte_offset);
                
                // Check for scroll update after sort
                if self.scroll_to_row.is_none() && !self.search_results.is_empty() && self.current_result_index == 0 {
                     let target_line = self.line_indexer.find_line_at_offset(self.search_results[0].byte_offset);
                     self.scroll_line = target_line;
                     self.scroll_to_row = Some(target_line);
                }
            }
        }
    }

    fn poll_replace_results(&mut self) {
        if !self.replace_in_progress {
            return;
        }

        let mut done = false;
        if let Some(ref rx) = self.replace_message_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    ReplaceMessage::Progress(processed, total) => {
                        let progress = processed as f32 / total as f32;
                        self.replace_progress = Some(progress);
                        self.replace_status_message = Some(format!("Replacing... {:.1}%", progress * 100.0));
                    }
                    ReplaceMessage::Done => {
                        self.replace_status_message = Some("Replacement complete.".to_string());
                        self.status_message = "Replacement complete.".to_string();
                        done = true;
                    }
                    ReplaceMessage::Error(e) => {
                        self.replace_status_message = Some(format!("Replace failed: {}", e));
                        self.status_message = format!("Replace failed: {}", e);
                        done = true;
                    }
                }
            }
        }

        if done {
            self.replace_in_progress = false;
            self.replace_message_rx = None;
            self.replace_cancellation_token = None;
            self.replace_progress = None;
        }
    }

    fn perform_replace(&mut self) {
        if self.replace_in_progress {
            return;
        }
        
        let Some(ref reader) = self.file_reader else { return };
        let input_path = reader.path().clone();
        
        // Ask for output file
        if let Some(output_path) = rfd::FileDialog::new()
            .set_file_name(&format!("{}.modified", input_path.file_name().unwrap().to_string_lossy()))
            .save_file() 
        {
            let query = self.search_query.clone();
            let replace_with = self.replace_query.clone();
            let use_regex = self.use_regex;
            
            let (tx, rx) = std::sync::mpsc::channel();
            self.replace_message_rx = Some(rx);
            self.replace_in_progress = true;
            self.replace_progress = Some(0.0);
            self.replace_status_message = None;
            
            let cancel_token = Arc::new(AtomicBool::new(false));
            self.replace_cancellation_token = Some(cancel_token.clone());
            
            std::thread::spawn(move || {
                Replacer::replace_all(
                    &input_path,
                    &output_path,
                    &query,
                    &replace_with,
                    use_regex,
                    tx,
                    cancel_token,
                );
            });
        }
    }

    fn go_to_next_result(&mut self) {
        if self.total_search_results == 0 {
            return;
        }

        let next_index = (self.current_result_index + 1) % self.total_search_results;
        
        // Check if next_index is within current page
        let page_end_index = self.search_page_start_index + self.search_results.len();
        
        if next_index >= self.search_page_start_index && next_index < page_end_index {
            // In current page
            self.current_result_index = next_index;
            let local_index = next_index - self.search_page_start_index;
            let result = &self.search_results[local_index];
            let target_line = self.line_indexer.find_line_at_offset(result.byte_offset);
            self.scroll_line = target_line;
            self.scroll_to_row = Some(target_line);
        } else {
            // Need to fetch next page
            // If we are wrapping around to 0
            if next_index == 0 {
                self.fetch_page(0, 0);
            } else {
                // Fetch next page starting from the end of current page
                // We need the byte offset to start searching from.
                // If we are just moving to the next page sequentially, we can use the last result's offset.
                if let Some(last_result) = self.search_results.last() {
                    // Start searching after the last result
                    // We should probably store the start offset of the next page if we knew it?
                    // But we don't. We just know we want the next 1000 results after the current last one.
                    let start_offset = last_result.byte_offset + 1; // +1 or +match_len? Regex find resumes after match.
                    // Actually, if we use `fetch_matches`, it uses `find_iter` which handles overlap.
                    // But we need to give it a start position.
                    // If we give `last_result.byte_offset + 1`, we might miss a match starting at `byte_offset + 1` if `match_len > 1`?
                    // No, if we found a match at `byte_offset`, the next match must start after it (or overlapping if regex allows, but `find_iter` is non-overlapping).
                    // So `byte_offset + match_len` is the correct resume point for `find_iter`.
                    // But `fetch_matches` takes a `start_offset` and treats it as the beginning of the search.
                    
                    // We should record the current page start offset before moving
                    if self.page_offsets.len() <= next_index / 1000 {
                         // This logic assumes pages are always 1000.
                         // But `search_results.len()` might be less if EOF.
                         // If we are here, it means we have more results (total > current page end).
                         // So we can assume we are fetching the next page.
                         // We should store the offset for the *current* page if not stored.
                         // But we want to store the offset for the *next* page so we can come back to it?
                         // No, `page_offsets` stores the start offset of each page.
                         // Page 0: 0.
                         // Page 1: offset X.
                         
                         // When we loaded Page 0, we didn't push 0 to `page_offsets`. We should.
                         if self.page_offsets.is_empty() {
                             self.page_offsets.push(0);
                         }
                         
                         // Now we are moving to Page N+1.
                         // The start offset for Page N+1 is `last_result.byte_offset + last_result.match_len`?
                         // Or just `last_result.byte_offset + 1`?
                         // Let's use `last_result.byte_offset + 1` to be safe, `find_iter` will skip if needed?
                         // Actually `find_iter` starts at the beginning of the string.
                         // If we pass a slice starting at `offset`, it finds matches in that slice.
                         // So yes, `last_result.byte_offset + 1` is safe, but `last_result.byte_offset + last_result.match_len` is more correct for non-overlapping.
                         // Let's use `last_result.byte_offset + 1` to be conservative.
                         
                         let next_page_start_offset = last_result.byte_offset + 1;
                         // We might need to store this to `page_offsets`?
                         // We can store it when we successfully load the page?
                         // Or store it now.
                         // But we don't know if we will find results.
                         // But we know `total_search_results` > `next_index`.
                    }
                    
                    let start_offset = last_result.byte_offset + 1;
                    self.fetch_page(next_index, start_offset);
                } else {
                    // Should not happen if total > 0
                    self.fetch_page(0, 0);
                }
            }
            self.current_result_index = next_index;
        }
    }

    fn go_to_previous_result(&mut self) {
        if self.total_search_results == 0 {
            return;
        }

        let prev_index = if self.current_result_index == 0 {
            self.total_search_results - 1
        } else {
            self.current_result_index - 1
        };

        // Check if prev_index is within current page
        let page_end_index = self.search_page_start_index + self.search_results.len();
        
        if prev_index >= self.search_page_start_index && prev_index < page_end_index {
            // In current page
            self.current_result_index = prev_index;
            let local_index = prev_index - self.search_page_start_index;
            let result = &self.search_results[local_index];
            let target_line = self.line_indexer.find_line_at_offset(result.byte_offset);
            self.scroll_line = target_line;
            self.scroll_to_row = Some(target_line);
        } else {
            // Need to fetch previous page (or last page if wrapping)
            if prev_index == self.total_search_results - 1 {
                // Wrapping to last page.
                // We don't know the offset of the last page easily unless we visited it.
                // But we can guess or just search from 0? No, that's slow.
                // If we haven't visited it, we can't jump to it efficiently without scanning.
                // But the user asked for "Next" optimization.
                // For "Previous" wrapping to end, we might have to disable it or warn?
                // Or just scan from 0 until we find it (might take time).
                // Let's just say "Cannot jump to end" or try to find it.
                // Or, since we know `total_results`, we can try to find the last page.
                // But we don't know where it starts.
                
                // For now, let's just reset to 0 if we can't find it?
                // Or better: If we have `page_offsets` for the target page, use it.
                // If not, maybe we shouldn't wrap?
                // Let's disable wrapping for now if we don't have the offset.
                // Or just fetch page 0.
                self.status_message = "Cannot wrap to end in paginated mode yet.".to_string();
                return;
            } else {
                // Fetch previous page
                // We need the start offset of the page containing `prev_index`.
                // We assume pages are 1000 items.
                let target_page_idx = prev_index / 1000;
                let target_page_start_index = target_page_idx * 1000;
                
                if let Some(&offset) = self.page_offsets.get(target_page_idx) {
                    self.fetch_page(target_page_start_index, offset);
                    self.current_result_index = prev_index;
                } else {
                    // We don't have the offset. This happens if we jumped around or haven't visited.
                    // But we should have visited previous pages to get here?
                    // Unless we jumped? We don't support jumping to arbitrary result index yet.
                    // So we should have it.
                    // But wait, `page_offsets` needs to be populated.
                    // I'll ensure `fetch_page` populates it.
                    
                    // If we are at page 1 (1000-1999) and go back to 999 (page 0).
                    // We should have `page_offsets[0]`.
                    
                    // Fallback: Search from 0?
                    self.fetch_page(0, 0);
                    self.current_result_index = 0; // Reset to 0 if lost
                }
            }
        }
    }

    fn fetch_page(&mut self, start_index: usize, start_offset: usize) {
        if self.search_in_progress {
            return;
        }
        
        let Some(ref reader) = self.file_reader else { return };
        
        self.search_results.clear();
        self.search_page_start_index = start_index;
        
        // Update page_offsets
        let page_idx = start_index / 1000;
        if page_idx >= self.page_offsets.len() {
            if page_idx == self.page_offsets.len() {
                self.page_offsets.push(start_offset);
            } else {
                // Gap in pages? Should not happen with sequential navigation.
                // But if it does, we can't easily fill it.
                // Just resize and fill with 0? No.
            }
        } else {
            // Update existing?
            self.page_offsets[page_idx] = start_offset;
        }

        let reader = reader.clone();
        let query = self.search_query.clone();
        let use_regex = self.use_regex;
        let (tx, rx) = std::sync::mpsc::sync_channel(10_000);
        self.search_message_rx = Some(rx);
        self.search_in_progress = true;
        
        let cancel_token = Arc::new(AtomicBool::new(false));
        self.search_cancellation_token = Some(cancel_token.clone());
        
        self.status_message = format!("Loading results {}...{}", start_index + 1, start_index + 1000);
        
        std::thread::spawn(move || {
            let mut engine = SearchEngine::new();
            engine.set_query(query, use_regex);
            engine.fetch_matches(reader, tx, start_offset, 1000, cancel_token);
        });
    }

    fn go_to_line(&mut self) {
        if let Ok(line_num) = self.goto_line_input.parse::<usize>() {
            if line_num > 0 && line_num <= self.line_indexer.total_lines() {
                let target_line = line_num - 1; // 0-indexed
                // Show a few lines of context above the target line for better orientation
                self.scroll_line = target_line.saturating_sub(3);
                self.scroll_to_row = Some(target_line);
                self.status_message = format!("Jumped to line {}", line_num);
            } else {
                self.status_message = "Line number out of range".to_string();
            }
        } else {
            self.status_message = "Invalid line number".to_string();
        }
    }

    fn render_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open...").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            // Auto-detect encoding
                            if let Ok(file) = std::fs::read(&path) {
                                let sample = &file[..file.len().min(4096)];
                                self.selected_encoding = detect_encoding(sample);
                            }
                            self.open_file(path);
                        }
                        ui.close_menu();
                    }
                    
                    if ui.button("File Info").clicked() {
                        self.show_file_info = !self.show_file_info;
                        ui.close_menu();
                    }
                    
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.wrap_mode, "Word Wrap");
                    ui.checkbox(&mut self.show_line_numbers, "Line Numbers");
                    ui.checkbox(&mut self.dark_mode, "Dark Mode");
                    
                    ui.separator();
                    
                    ui.label("Font Size:");
                    ui.add(egui::Slider::new(&mut self.font_size, 8.0..=32.0));
                    
                    ui.separator();
                    
                    if ui.button("Select Encoding").clicked() {
                        self.show_encoding_selector = true;
                        ui.close_menu();
                    }
                });

                ui.menu_button("Search", |ui| {
                    ui.checkbox(&mut self.use_regex, "Use Regex");
                    if ui.checkbox(&mut self.show_replace, "Show Replace").changed() {
                        // Reset replace state if hidden? No, keep it.
                    }
                });

                ui.menu_button("Tools", |ui| {
                    if ui.checkbox(&mut self.tail_mode, "Tail Mode (Auto-refresh)").changed() {
                        if self.tail_mode {
                            self.setup_file_watcher();
                        } else {
                            self.watcher = None;
                            self.file_change_rx = None;
                        }
                    }
                });
            });
        });
    }

    fn render_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .desired_width(300.0)
                );
                
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.perform_search(false);
                }
                
                if ui.add_enabled(!self.search_in_progress, egui::Button::new("🔍 Find")).clicked() {
                    self.perform_search(false);
                }

                if ui.add_enabled(!self.search_in_progress, egui::Button::new("🔎 Find All")).clicked() {
                    self.perform_search(true);
                }
                
                if ui.button("⬆ Previous").clicked() {
                    self.go_to_previous_result();
                }
                
                if ui.button("⬇ Next").clicked() {
                    self.go_to_next_result();
                }

                if self.search_in_progress {
                    ui.add(egui::Spinner::new().size(18.0));
                    ui.label("Searching...");
                    if ui.button("Stop").clicked() {
                        if let Some(token) = &self.search_cancellation_token {
                            token.store(true, Ordering::Relaxed);
                        }
                        self.search_in_progress = false;
                        self.status_message = "Search stopped by user".to_string();
                    }
                }
                
                let total_results = self.total_search_results;
                if total_results > 0 {
                    // Show current position over total
                    let current = (self.current_result_index + 1).min(total_results);
                    ui.label(format!("{}/{}", current, total_results));
                }
                
                ui.separator();
                
                ui.label("Go to line:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.goto_line_input)
                        .desired_width(80.0)
                );
                
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.go_to_line();
                }
                
                if ui.button("Go").clicked() {
                    self.go_to_line();
                }
            });

            if self.show_replace {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Replace with:");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.replace_query)
                            .desired_width(200.0)
                            .hint_text("Replacement text...")
                    );

                    if self.replace_in_progress {
                        if ui.button("Stop Replace").clicked() {
                            if let Some(token) = &self.replace_cancellation_token {
                                token.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        ui.spinner();
                        if let Some(progress) = self.replace_progress {
                            ui.label(format!("{:.1}%", progress * 100.0));
                        }
                    } else {
                        if ui.button("Replace All").clicked() {
                            self.perform_replace();
                        }
                    }
                });
                
                if let Some(ref msg) = self.replace_status_message {
                    ui.label(msg);
                }
            }
            
            if let Some(ref error) = self.search_error {
                ui.colored_label(egui::Color32::RED, format!("Search error: {}", error));
            }
        });
    }

    fn render_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(ref reader) = self.file_reader {
                    ui.label(format!("File: {}", reader.path().display()));
                    ui.separator();
                    ui.label(format!("Size: {} bytes", reader.len()));
                    ui.separator();
                    ui.label(format!("Lines: ~{}", self.line_indexer.total_lines()));
                    ui.separator();
                    ui.label(format!("Encoding: {}", reader.encoding().name()));
                    ui.separator();
                    ui.label(format!("Line: {}", self.scroll_line + 1));
                } else {
                    ui.label("No file opened - Click File → Open to start");
                }
                
                if !self.status_message.is_empty() {
                    ui.separator();
                    ui.label(&self.status_message);
                }
            });
        });
    }

    fn render_text_area(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref reader) = self.file_reader {
                let available_height = ui.available_height();
                let font_id = egui::FontId::monospace(self.font_size);
                let line_height = ui.fonts(|f| f.row_height(&font_id));
                self.visible_lines = ((available_height / line_height).ceil() as usize).saturating_add(2);

                let mut scroll_area = if self.wrap_mode {
                    egui::ScrollArea::vertical()
                } else {
                    egui::ScrollArea::both()
                }
                    // Tie scroll memory to the current file path so new files start at the top
                    .id_salt(
                        self.file_reader
                            .as_ref()
                            .map(|r| r.path().display().to_string())
                            .unwrap_or_else(|| "no_file".to_string())
                    )
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .drag_to_scroll(true);
                
                // Apply programmatic scroll if requested
                if let Some(target_row) = self.scroll_to_row.take() {
                    scroll_area = scroll_area.vertical_scroll_offset(target_row as f32 * line_height);
                }
                
                let mut first_visible_row = None;
                
                scroll_area.show_rows(
                        ui,
                        line_height,
                        self.line_indexer.total_lines(),
                        |ui, row_range| {
                            // Capture the first visible row
                            if first_visible_row.is_none() {
                                first_visible_row = row_range.clone().next();
                            }
                            
                            for line_num in row_range {
                                // Use get_line_with_reader for sparse index support
                                if let Some((start, end)) = self.line_indexer.get_line_with_reader(line_num, reader) {
                                    let line_text = reader.get_chunk(start, end);
                                    let line_text = line_text.trim_end_matches('\n').trim_end_matches('\r');

                                    // Collect matches that fall within this line's byte span; this works even with sparse line indexing
                                    let mut line_matches: Vec<(usize, usize, bool)> = Vec::new();
                                    
                                    // Use binary search to find the first potential match
                                    // This assumes search_results is sorted by byte_offset
                                    let start_idx = self.search_results.partition_point(|r| r.byte_offset < start);
                                    
                                    for (idx, res) in self.search_results.iter().enumerate().skip(start_idx) {
                                        if res.byte_offset >= end {
                                            break;
                                        }

                                        let rel_start = res.byte_offset.saturating_sub(start);
                                        if rel_start >= line_text.len() {
                                            continue;
                                        }
                                        let rel_end = (rel_start + res.match_len).min(line_text.len());
                                        
                                        // Check if this is the currently selected result
                                        // We need to map local index to global index
                                        let global_idx = self.search_page_start_index + idx;
                                        let is_selected = global_idx == self.current_result_index;
                                        
                                        line_matches.push((rel_start, rel_end, is_selected));
                                    }
                                    
                                    ui.horizontal(|ui| {
                                        if self.show_line_numbers {
                                            let ln_text = egui::RichText::new(format!("{:6} ", line_num + 1))
                                                .monospace()
                                                .color(egui::Color32::DARK_GRAY);
                                            // Make line numbers non-selectable so drag-select only captures the content text
                                            ui.add(egui::Label::new(ln_text).selectable(false));
                                        }
                                        
                                        // Build label with highlighted search matches
                                        let label = if !line_matches.is_empty() {
                                            // Create a LayoutJob to highlight matches within the line using their byte offsets
                                            let mut job = egui::text::LayoutJob::default();
                                            let mut last_end = 0;

                                            for (abs_start, abs_end, is_selected) in line_matches.iter() {
                                                if *abs_start > last_end {
                                                    job.append(
                                                        &line_text[last_end..*abs_start],
                                                        0.0,
                                                        egui::TextFormat {
                                                            font_id: egui::FontId::monospace(self.font_size),
                                                            color: if self.dark_mode { egui::Color32::LIGHT_GRAY } else { egui::Color32::BLACK },
                                                            ..Default::default()
                                                        },
                                                    );
                                                }

                                                let match_end = (*abs_end).min(line_text.len());
                                                job.append(
                                                    &line_text[*abs_start..match_end],
                                                    0.0,
                                                    egui::TextFormat {
                                                        font_id: egui::FontId::monospace(self.font_size),
                                                        color: egui::Color32::BLACK,
                                                        background: if *is_selected {
                                                            egui::Color32::from_rgb(255, 200, 0) // orange-ish for current match
                                                        } else {
                                                            egui::Color32::YELLOW
                                                        },
                                                        ..Default::default()
                                                    },
                                                );

                                                last_end = match_end;
                                            }

                                            // Add remaining text after last match
                                            if last_end < line_text.len() {
                                                job.append(
                                                    &line_text[last_end..],
                                                    0.0,
                                                    egui::TextFormat {
                                                        font_id: egui::FontId::monospace(self.font_size),
                                                        color: if self.dark_mode { egui::Color32::LIGHT_GRAY } else { egui::Color32::BLACK },
                                                        ..Default::default()
                                                    },
                                                );
                                            }

                                            if self.wrap_mode {
                                                job.wrap = egui::text::TextWrapping {
                                                    max_width: ui.available_width(),
                                                    ..Default::default()
                                                };
                                            }

                                            ui.add(egui::Label::new(job).extend())
                                        } else {
                                            let text = egui::RichText::new(line_text)
                                                .monospace()
                                                .size(self.font_size);
                                            
                                            // Apply wrap mode
                                            if self.wrap_mode {
                                                ui.add(egui::Label::new(text).wrap())
                                            } else {
                                                ui.add(egui::Label::new(text).extend())
                                            }
                                        };
                                        
                                        // Enable text selection for copy-paste
                                        if label.hovered() {
                                            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Text);
                                        }
                                        
                                        // Ensure labels don't consume scroll events
                                        label.surrender_focus();
                                    });
                                }
                            }
                        },
                    );
                
                // Update scroll_line to match what was actually displayed
                if let Some(first_row) = first_visible_row {
                    self.scroll_line = first_row;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.heading("Large Text Viewer");
                    ui.label("\nClick File → Open to load a text file");
                });
            }
        });
    }

    fn render_encoding_selector(&mut self, ctx: &egui::Context) {
        if self.show_encoding_selector {
            egui::Window::new("Select Encoding")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    for (name, encoding) in available_encodings() {
                        if ui.selectable_label(
                            std::ptr::eq(self.selected_encoding, encoding),
                            name
                        ).clicked() {
                            self.selected_encoding = encoding;
                            
                            // Reload file with new encoding
                            if let Some(ref reader) = self.file_reader {
                                let path = reader.path().clone();
                                self.open_file(path);
                            }
                            
                            self.show_encoding_selector = false;
                        }
                    }
                    
                    if ui.button("Cancel").clicked() {
                        self.show_encoding_selector = false;
                    }
                });
        }
    }

    fn render_file_info(&mut self, ctx: &egui::Context) {
        if self.show_file_info {
            if let Some(ref reader) = self.file_reader {
                egui::Window::new("File Information")
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(format!("Path: {}", reader.path().display()));
                        ui.label(format!("Size: {} bytes ({:.2} MB)", 
                            reader.len(), 
                            reader.len() as f64 / 1_000_000.0
                        ));
                        ui.label(format!("Lines: ~{}", self.line_indexer.total_lines()));
                        ui.label(format!("Encoding: {}", reader.encoding().name()));
                        
                        if ui.button("Close").clicked() {
                            self.show_file_info = false;
                        }
                    });
            }
        }
    }
}

impl eframe::App for TextViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Set theme
        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        // Check for file changes in tail mode
        if self.tail_mode {
            self.check_file_changes();
            ctx.request_repaint(); // Keep refreshing
        }

        self.poll_search_results();
        self.poll_replace_results();

        if self.search_in_progress || self.replace_in_progress {
            ctx.request_repaint(); // Keep spinner animated during long searches
        }

        self.render_menu_bar(ctx);
        self.render_toolbar(ctx);
        self.render_status_bar(ctx);
        self.render_text_area(ctx);
        self.render_encoding_selector(ctx);
        self.render_file_info(ctx);
    }
}
