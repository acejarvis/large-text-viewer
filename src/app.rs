use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use notify::{Watcher, RecursiveMode, Result as NotifyResult};
use encoding_rs::Encoding;

use crate::file_reader::{FileReader, detect_encoding, available_encodings};
use crate::line_indexer::LineIndexer;
use crate::search_engine::{SearchEngine, SearchResult, SearchMessage};

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
    use_regex: bool,
    search_results: Vec<SearchResult>,
    current_result_index: usize,
    search_error: Option<String>,
    search_in_progress: bool,
    search_find_all: bool,
    search_message_rx: Option<Receiver<SearchMessage>>,
    
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
            use_regex: false,
            search_results: Vec::new(),
            current_result_index: 0,
            search_error: None,
            search_in_progress: false,
            search_find_all: true,
            search_message_rx: None,
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

        let reader = reader.clone();
        let query = self.search_query.clone();
        let use_regex = self.use_regex;
        let max_results = if find_all { usize::MAX } else { 1 };
        // Use a bounded channel to provide backpressure to search threads
        // This prevents memory explosion if the UI thread can't keep up with results
        let (tx, rx) = std::sync::mpsc::sync_channel(10_000);

        self.search_message_rx = Some(rx);
        self.search_in_progress = true;
        self.search_find_all = find_all;
        self.status_message = if find_all {
            "Searching all matches...".to_string()
        } else {
            "Searching first match...".to_string()
        };

        self.search_engine.set_query(query, use_regex);
        
        // Correct approach:
        // 1. Configure the search engine on the main thread (already done with set_query)
        // 2. Call search_parallel which spawns threads and returns immediately
        
        self.search_engine.search_parallel(reader, tx, max_results);
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
                    SearchMessage::ChunkResult(chunk_result) => {
                        // Add results
                        self.search_results.extend(chunk_result.matches);
                        new_results_added = true;
                        
                        let total = self.search_results.len();
                        self.status_message = format!("Found {} matches...", total);
                        
                        // If we found results and haven't scrolled yet, scroll to the first one
                        if total > 0 && self.scroll_to_row.is_none() && self.current_result_index == 0 {
                             // We need to sort at least once to find the true first result
                             // But doing it here might be expensive if we do it often.
                             // For the very first result, we can just check if we have any.
                             // However, to be correct, we should probably wait or do a partial check.
                             // For now, let's defer the sort to outside the loop.
                        }
                    }
                    SearchMessage::Done => {
                        self.search_in_progress = false;
                        self.search_message_rx = None;
                        
                        // Final sort to ensure everything is in order
                        self.search_results.sort_by_key(|r| r.byte_offset);
                        
                        let total = self.search_results.len();
                        if total > 0 {
                            if self.search_find_all {
                                self.status_message = format!("Found {} matches", total);
                            } else {
                                self.status_message = "Showing first match. Run Find All to see every result.".to_string();
                            }
                            
                            // Ensure we scroll to the first result if we haven't yet
                            if self.scroll_to_row.is_none() {
                                 let target_line = self.line_indexer.find_line_at_offset(self.search_results[0].byte_offset);
                                 self.scroll_line = target_line;
                                 self.scroll_to_row = Some(target_line);
                            }
                        } else {
                            self.status_message = "No matches found".to_string();
                        }
                        return; // Stop processing messages
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

    fn go_to_next_result(&mut self) {
        if !self.search_results.is_empty() {
            self.current_result_index = (self.current_result_index + 1) % self.search_results.len();
            let result = &self.search_results[self.current_result_index];
            let target_line = self.line_indexer.find_line_at_offset(result.byte_offset);
            self.scroll_line = target_line;
            self.scroll_to_row = Some(target_line);
        }
    }

    fn go_to_previous_result(&mut self) {
        if !self.search_results.is_empty() {
            self.current_result_index = if self.current_result_index == 0 {
                self.search_results.len() - 1
            } else {
                self.current_result_index - 1
            };
            let result = &self.search_results[self.current_result_index];
            let target_line = self.line_indexer.find_line_at_offset(result.byte_offset);
            self.scroll_line = target_line;
            self.scroll_to_row = Some(target_line);
        }
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
                }
                
                let total_results = self.search_results.len();
                if total_results > 0 {
                    // Show current position over total, even if we stored fewer than total
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
                                        line_matches.push((rel_start, rel_end, idx == self.current_result_index));
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

        if self.search_in_progress {
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
