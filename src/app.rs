use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use notify::{Watcher, RecursiveMode, Result as NotifyResult};
use encoding_rs::Encoding;

use crate::file_reader::{FileReader, detect_encoding, available_encodings};
use crate::line_indexer::LineIndexer;
use crate::search_engine::{SearchEngine, SearchResult};

pub struct TextViewerApp {
    file_reader: Option<FileReader>,
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
    
    // Selection state for copy-paste
    selection_start: Option<(usize, usize)>, // (line, column)
    selection_end: Option<(usize, usize)>,
    
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
            goto_line_input: String::new(),
            show_file_info: false,
            tail_mode: false,
            watcher: None,
            file_change_rx: None,
            status_message: String::new(),
            selected_encoding: encoding_rs::UTF_8,
            show_encoding_selector: false,
            selection_start: None,
            selection_end: None,
            scroll_to_row: None,
        }
    }
}

impl TextViewerApp {
    fn open_file(&mut self, path: PathBuf) {
        match FileReader::new(path.clone(), self.selected_encoding) {
            Ok(reader) => {
                self.file_reader = Some(reader);
                self.line_indexer.index_file(self.file_reader.as_ref().unwrap());
                self.scroll_line = 0;
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

    fn perform_search(&mut self) {
        self.search_error = None;
        self.search_results.clear();
        self.current_result_index = 0;

        if let Some(ref reader) = self.file_reader {
            self.search_engine.set_query(self.search_query.clone(), self.use_regex);
            
            match self.search_engine.search(reader, 10000) {
                Ok(_) => {
                    self.search_results = self.search_engine.results().to_vec();
                    if !self.search_results.is_empty() {
                        let target_line = self.search_results[0].line_number; // Show first result at top
                        self.scroll_line = target_line;
                        self.scroll_to_row = Some(target_line);
                        self.status_message = format!("Found {} matches", self.search_results.len());
                    } else {
                        self.status_message = "No matches found".to_string();
                    }
                }
                Err(e) => {
                    self.search_error = Some(e);
                }
            }
        }
    }

    fn go_to_next_result(&mut self) {
        if !self.search_results.is_empty() {
            self.current_result_index = (self.current_result_index + 1) % self.search_results.len();
            let result = &self.search_results[self.current_result_index];
            let target_line = result.line_number; // Show search result line at top
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
            let target_line = result.line_number; // Show search result line at top
            self.scroll_line = target_line;
            self.scroll_to_row = Some(target_line);
        }
    }

    fn go_to_line(&mut self) {
        if let Ok(line_num) = self.goto_line_input.parse::<usize>() {
            if line_num > 0 && line_num <= self.line_indexer.total_lines() {
                let target_line = line_num - 1; // 0-indexed, show at top of viewport
                self.scroll_line = target_line;
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
                    self.perform_search();
                }
                
                if ui.button("🔍 Find").clicked() {
                    self.perform_search();
                }
                
                if ui.button("⬆ Previous").clicked() {
                    self.go_to_previous_result();
                }
                
                if ui.button("⬇ Next").clicked() {
                    self.go_to_next_result();
                }
                
                if !self.search_results.is_empty() {
                    ui.label(format!("{}/{}", self.current_result_index + 1, self.search_results.len()));
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
                let line_height = self.font_size * 1.5;
                self.visible_lines = (available_height / line_height) as usize + 2;

                let mut scroll_area = egui::ScrollArea::vertical()
                    .id_salt("text_scroll_area")
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
                                    
                                    ui.horizontal(|ui| {
                                        if self.show_line_numbers {
                                            ui.label(
                                                egui::RichText::new(format!("{:6} ", line_num + 1))
                                                    .monospace()
                                                    .color(egui::Color32::DARK_GRAY)
                                            );
                                        }
                                        
                                        let mut text = egui::RichText::new(line_text)
                                            .monospace()
                                            .size(self.font_size);
                                        
                                        // Highlight search results
                                        if self.search_results.iter().any(|r| r.line_number == line_num) {
                                            text = text.background_color(egui::Color32::from_rgb(100, 100, 0));
                                        }
                                        
                                        let label = ui.label(text);
                                        
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

        self.render_menu_bar(ctx);
        self.render_toolbar(ctx);
        self.render_status_bar(ctx);
        self.render_text_area(ctx);
        self.render_encoding_selector(ctx);
        self.render_file_info(ctx);
    }
}
