use core::error;
use ratatui::{
    Terminal, backend::CrosstermBackend, crossterm::event::{self, Event, KeyCode}, layout::{Constraint, Direction, Layout, Rect, Size}, style::{Color, Modifier, Style}, widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{time::Duration};
use tokio::sync::mpsc;
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::events::DisplayEvent;

pub struct Interface {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    prompt_tx: mpsc::UnboundedSender<String>,
    event_rx: mpsc::UnboundedReceiver<DisplayEvent>,
    input: String,
    history: Vec<DisplayEvent>,
    history_scroll_state: ScrollViewState,
}

impl Interface {
    pub fn new(
        prompt_tx: mpsc::UnboundedSender<String>,
        event_rx: mpsc::UnboundedReceiver<DisplayEvent>,
    ) -> Self {
        Self {
            terminal: ratatui::init(),
            prompt_tx,
            event_rx,
            input: String::new(),
            history: Vec::<DisplayEvent>::new(),
            history_scroll_state: ScrollViewState::default(),
        }
    }
}

impl Interface {
    pub fn render(&mut self) -> Result<(), Box<dyn error::Error>> {
        self.terminal.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Max(3)])
                .split(area);
            let history_chunk = chunks[0];
            let prompt_chunk = chunks[1];
            let input_widget = Paragraph::new(format!("❯ {}", self.input))
                .block(Block::default().borders(Borders::ALL).title("Prompt"));
            frame.render_widget(input_widget, prompt_chunk);
            let history_width = history_chunk.width.max(1);
            let mut history_total_height = 0;
            let history_texts = self
                .history
                .iter()
                .map(|item| match item {
                        DisplayEvent::User(chunk) => (format!("❯ {}", chunk), Style::default().bg(Color::Rgb(31,31,31))),
                        DisplayEvent::Content(chunk) => (chunk.to_string(), Style::default()),
                        DisplayEvent::Reasoning(chunk)  => (chunk.to_string(), Style::default().add_modifier(Modifier::ITALIC).dim()),
                        DisplayEvent::Error(chunk)  => (chunk.to_string(), Style::default().red()),
                        DisplayEvent::ToolCall(tool) => (format!("Tool:{} Arguments:{}", tool.name, tool.arguments), Style::default()),
                    })
                .collect::<Vec<(String, Style)>>();

            let history_widgets = history_texts.iter().map(|(text, style)| {
                let paragraph = Paragraph::new(tui_markdown::from_str(text)).style(*style).wrap(Wrap { trim: false });
                history_total_height += paragraph.line_count(history_width);
                paragraph
            }).collect::<Vec<Paragraph>>();
            let history_content_size = Size::new(history_width, history_total_height as u16);
            let mut history_scroll_view = ScrollView::new(history_content_size)
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);
            let mut curr_height = 0;
            for item in history_widgets.iter() {
                let item_height = item.line_count(history_width) as u16;
                history_scroll_view.render_widget(
                    item,
                    Rect::new(
                        0,
                        curr_height,
                        history_content_size.width,
                        item_height,
                    ),
                );
                curr_height += item_height;
            }
            frame.render_stateful_widget(
                history_scroll_view,
                history_chunk,
                &mut self.history_scroll_state,
            );
        })?;

        return Ok(());
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn error::Error>> {
        loop {
            self.render()?;

            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(&event);
            }

            while event::poll(Duration::from_millis(0))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Esc => {
                            ratatui::restore();
                            return Ok(());
                        }
                        KeyCode::Char(char) => self.input.push(char),
                        KeyCode::Backspace => {
                            self.input.pop();
                        }
                        KeyCode::Enter => {
                            self.prompt_tx.send(self.input.to_string()).ok();
                            self.history.push(DisplayEvent::User(self.input.to_string()));
                            self.input.clear();
                        }
                        KeyCode::Up => {
                            self.history_scroll_state.scroll_up();
                        }
                        KeyCode::Down => {
                            self.history_scroll_state.scroll_down();
                        }
                        _ => {}
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    }

    fn handle_event(&mut self, event: &DisplayEvent) {
        if let Some(last) = self.history.last_mut() {
            match (event, last) {
                (DisplayEvent::User(new), DisplayEvent::User(str))
                | (DisplayEvent::Content(new), DisplayEvent::Content(str))
                | (DisplayEvent::Reasoning(new), DisplayEvent::Reasoning(str))
                | (DisplayEvent::Error(new), DisplayEvent::Error(str)) => {
                    str.push_str(new);
                    self.history_scroll_state.scroll_to_bottom();
                    return;
                }
                _ => {}
            }
        }
        self.history.push(event.clone());
    }
}
