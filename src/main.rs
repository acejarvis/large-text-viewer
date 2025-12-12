mod editor;
mod file_handler;
mod search;

use iced::widget::{button, column, container, row, scrollable, slider, text, text_input};
use iced::{keyboard, Application, Command, Element, Event, Length, Settings, Subscription, Theme};
use std::path::PathBuf;

// Scrolling constants
const LINES_PER_WHEEL_TICK: f32 = 3.0;
const PIXELS_PER_LINE: f32 = 20.0;

// Slider constants
const SLIDER_WIDTH: f32 = 200.0;
const SLIDER_STEP: f32 = 0.001;

pub fn main() -> iced::Result {
    LargeTextFileViewer::run(Settings::default())
}

#[derive(Debug, Clone)]
pub enum Message {
    OpenFile,
    FileOpened(Result<PathBuf, String>),
    ScrollTo(usize),
    ScrollBy(i32), // Scroll by a number of lines (positive = down, negative = up)
    ScrollToPosition(f32), // Scroll to a position (0.0 to 1.0)
    EventOccurred(Event),
    Search(String),
    SearchNext,
    SearchPrevious,
    SearchComplete(Vec<usize>),
    Replace(String),
    ReplaceAll,
    ReplaceComplete(Result<(), String>),
    EditLine(usize, String),
    SaveFile,
    FileSaved(Result<(), String>),
}

struct LargeTextFileViewer {
    file_handler: Option<file_handler::FileHandler>,
    viewport_start: usize,
    viewport_size: usize,
    search_query: String,
    search_results: Vec<usize>,
    current_search_index: Option<usize>,
    replace_text: String,
    status_message: String,
    file_path: Option<PathBuf>,
}

impl Default for LargeTextFileViewer {
    fn default() -> Self {
        Self {
            file_handler: None,
            viewport_start: 0,
            viewport_size: 50, // Display 50 lines at a time
            search_query: String::new(),
            search_results: Vec::new(),
            current_search_index: None,
            replace_text: String::new(),
            status_message: String::from("No file loaded"),
            file_path: None,
        }
    }
}

impl LargeTextFileViewer {
    fn scroll_by(&mut self, lines: i32) -> Command<Message> {
        if let Some(handler) = &self.file_handler {
            let total_lines = handler.total_lines();
            let new_start = if lines < 0 {
                self.viewport_start.saturating_sub(lines.unsigned_abs() as usize)
            } else {
                self.viewport_start.saturating_add(lines as usize)
            };
            self.viewport_start = new_start.min(total_lines.saturating_sub(self.viewport_size));
        }
        Command::none()
    }
}

impl Application for LargeTextFileViewer {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        (Self::default(), Command::none())
    }

    fn title(&self) -> String {
        String::from("Large Text File Viewer")
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        iced::event::listen().map(Message::EventOccurred)
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::EventOccurred(event) => {
                if let Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers: _,
                    ..
                }) = event
                {
                    match key.as_ref() {
                        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                            return self.scroll_by(-1);
                        }
                        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                            return self.scroll_by(1);
                        }
                        keyboard::Key::Named(keyboard::key::Named::PageUp) => {
                            return self.scroll_by(-(self.viewport_size as i32));
                        }
                        keyboard::Key::Named(keyboard::key::Named::PageDown) => {
                            return self.scroll_by(self.viewport_size as i32);
                        }
                        keyboard::Key::Named(keyboard::key::Named::Home) => {
                            if self.file_handler.is_some() {
                                self.viewport_start = 0;
                            }
                            return Command::none();
                        }
                        keyboard::Key::Named(keyboard::key::Named::End) => {
                            if let Some(handler) = &self.file_handler {
                                let total_lines = handler.total_lines();
                                self.viewport_start =
                                    total_lines.saturating_sub(self.viewport_size);
                            }
                            return Command::none();
                        }
                        _ => {}
                    }
                } else if let Event::Mouse(mouse_event) = event {
                    if let iced::mouse::Event::WheelScrolled { delta } = mouse_event {
                        let scroll_lines = match delta {
                            iced::mouse::ScrollDelta::Lines { y, .. } => {
                                (-y * LINES_PER_WHEEL_TICK).clamp(i32::MIN as f32, i32::MAX as f32) as i32
                            }
                            iced::mouse::ScrollDelta::Pixels { y, .. } => {
                                (-y / PIXELS_PER_LINE).clamp(i32::MIN as f32, i32::MAX as f32) as i32
                            }
                        };
                        return self.scroll_by(scroll_lines);
                    }
                }
                Command::none()
            }
            Message::ScrollBy(lines) => self.scroll_by(lines),
            Message::ScrollToPosition(position) => {
                if let Some(handler) = &self.file_handler {
                    let total_lines = handler.total_lines();
                    let max_scroll_position = total_lines.saturating_sub(self.viewport_size);
                    if max_scroll_position > 0 {
                        let target_line = (position * max_scroll_position as f32) as usize;
                        self.viewport_start = target_line.min(max_scroll_position);
                    }
                }
                Command::none()
            }
            Message::OpenFile => Command::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                        .ok_or_else(|| "No file selected".to_string())
                },
                Message::FileOpened,
            ),
            Message::FileOpened(result) => {
                match result {
                    Ok(path) => match file_handler::FileHandler::new(&path) {
                        Ok(handler) => {
                            self.file_path = Some(path.clone());
                            self.file_handler = Some(handler);
                            self.viewport_start = 0;
                            self.status_message = format!(
                                "Loaded: {} ({} lines)",
                                path.display(),
                                self.file_handler.as_ref().unwrap().total_lines()
                            );
                        }
                        Err(e) => {
                            self.status_message = format!("Error loading file: {}", e);
                        }
                    },
                    Err(e) => {
                        self.status_message = e;
                    }
                }
                Command::none()
            }
            Message::ScrollTo(line) => {
                if let Some(handler) = &self.file_handler {
                    let total_lines = handler.total_lines();
                    self.viewport_start = line.min(total_lines.saturating_sub(self.viewport_size));
                }
                Command::none()
            }
            Message::Search(query) => {
                self.search_query = query.clone();
                if let Some(handler) = &self.file_handler {
                    let handler_clone = handler.clone();
                    Command::perform(
                        async move { search::search_file(&handler_clone, &query).await },
                        Message::SearchComplete,
                    )
                } else {
                    Command::none()
                }
            }
            Message::SearchNext => {
                if !self.search_results.is_empty() {
                    let next_index = match self.current_search_index {
                        Some(idx) => (idx + 1) % self.search_results.len(),
                        None => 0,
                    };
                    self.current_search_index = Some(next_index);
                    if let Some(&line) = self.search_results.get(next_index) {
                        self.viewport_start = line.saturating_sub(self.viewport_size / 2);
                    }
                }
                Command::none()
            }
            Message::SearchPrevious => {
                if !self.search_results.is_empty() {
                    let prev_index = match self.current_search_index {
                        Some(idx) if idx > 0 => idx - 1,
                        _ => self.search_results.len() - 1,
                    };
                    self.current_search_index = Some(prev_index);
                    if let Some(&line) = self.search_results.get(prev_index) {
                        self.viewport_start = line.saturating_sub(self.viewport_size / 2);
                    }
                }
                Command::none()
            }
            Message::SearchComplete(results) => {
                self.search_results = results;
                self.current_search_index = if self.search_results.is_empty() {
                    None
                } else {
                    Some(0)
                };
                self.status_message = format!("Found {} matches", self.search_results.len());
                Command::none()
            }
            Message::Replace(text) => {
                self.replace_text = text;
                Command::none()
            }
            Message::ReplaceAll => {
                if let Some(handler) = &mut self.file_handler {
                    if let Some(path) = &self.file_path {
                        let path_clone = path.clone();
                        let search_query = self.search_query.clone();
                        let replace_text = self.replace_text.clone();
                        let handler_clone = handler.clone();

                        Command::perform(
                            async move {
                                editor::replace_all(
                                    &handler_clone,
                                    &path_clone,
                                    &search_query,
                                    &replace_text,
                                )
                                .await
                            },
                            Message::ReplaceComplete,
                        )
                    } else {
                        Command::none()
                    }
                } else {
                    Command::none()
                }
            }
            Message::ReplaceComplete(result) => {
                match result {
                    Ok(()) => {
                        self.status_message = "Replace completed successfully".to_string();
                        // Reload the file to reflect changes
                        if let Some(path) = &self.file_path {
                            match file_handler::FileHandler::new(path) {
                                Ok(handler) => {
                                    self.file_handler = Some(handler);
                                }
                                Err(e) => {
                                    self.status_message = format!("Error reloading file: {}", e);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        self.status_message = format!("Replace error: {}", e);
                    }
                }
                Command::none()
            }
            Message::EditLine(line_num, new_content) => {
                if let Some(handler) = &mut self.file_handler {
                    handler.update_line(line_num, new_content);
                }
                Command::none()
            }
            Message::SaveFile => {
                if let Some(handler) = &self.file_handler {
                    if let Some(path) = &self.file_path {
                        let path_clone = path.clone();
                        let handler_clone = handler.clone();

                        Command::perform(
                            async move { editor::save_file(&handler_clone, &path_clone).await },
                            Message::FileSaved,
                        )
                    } else {
                        Command::none()
                    }
                } else {
                    Command::none()
                }
            }
            Message::FileSaved(result) => {
                match result {
                    Ok(()) => {
                        self.status_message = "File saved successfully".to_string();
                    }
                    Err(e) => {
                        self.status_message = format!("Save error: {}", e);
                    }
                }
                Command::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let toolbar = row![
            button("Open File").on_press(Message::OpenFile),
            button("Save File").on_press(Message::SaveFile),
        ]
        .spacing(10)
        .padding(10);

        let search_bar = row![
            text("Search:"),
            text_input("Enter search query...", &self.search_query)
                .on_input(Message::Search)
                .width(Length::Fixed(300.0)),
            button("Previous").on_press(Message::SearchPrevious),
            button("Next").on_press(Message::SearchNext),
        ]
        .spacing(10)
        .padding(10);

        let replace_bar = row![
            text("Replace:"),
            text_input("Replace with...", &self.replace_text)
                .on_input(Message::Replace)
                .width(Length::Fixed(300.0)),
            button("Replace All").on_press(Message::ReplaceAll),
        ]
        .spacing(10)
        .padding(10);

        let content_view = if let Some(handler) = &self.file_handler {
            let lines = handler.get_viewport_lines(self.viewport_start, self.viewport_size);

            let mut line_widgets = Vec::new();
            for (idx, line) in lines.iter().enumerate() {
                let line_num = self.viewport_start + idx;
                let is_match = self.search_results.contains(&line_num);

                let line_text = if is_match {
                    text(format!("{}: {} [MATCH]", line_num + 1, line)).size(14)
                } else {
                    text(format!("{}: {}", line_num + 1, line)).size(14)
                };

                line_widgets.push(line_text.into());
            }

            scrollable(column(line_widgets).spacing(2))
                .direction(scrollable::Direction::Vertical(
                    scrollable::Properties::default()
                ))
                .height(Length::Fill)
        } else {
            scrollable(
                column![text("No file loaded. Click 'Open File' to get started.")].padding(20),
            )
            .direction(scrollable::Direction::Vertical(
                scrollable::Properties::default()
            ))
            .height(Length::Fill)
        };

        let status_bar = container(text(&self.status_message).size(12))
            .padding(5)
            .width(Length::Fill);

        let navigation_bar = if let Some(handler) = &self.file_handler {
            let total_lines = handler.total_lines();
            let end_line = (self.viewport_start + self.viewport_size).min(total_lines);
            
            // Calculate slider position (0.0 to 1.0) based on the scrollable range
            let max_scroll_position = total_lines.saturating_sub(self.viewport_size);
            let slider_position = if max_scroll_position > 0 {
                self.viewport_start as f32 / max_scroll_position as f32
            } else {
                0.0
            };

            row![
                button("Top").on_press(Message::ScrollTo(0)),
                text(format!(
                    "Lines {}-{} of {}",
                    self.viewport_start + 1,
                    end_line,
                    total_lines
                )),
                text("Position:"),
                slider(0.0..=1.0, slider_position, Message::ScrollToPosition)
                    .width(Length::Fixed(SLIDER_WIDTH))
                    .step(SLIDER_STEP),
            ]
            .spacing(10)
            .padding(10)
        } else {
            row![].spacing(10).padding(10)
        };

        container(
            column![
                toolbar,
                search_bar,
                replace_bar,
                navigation_bar,
                content_view,
                status_bar,
            ]
            .spacing(0),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}