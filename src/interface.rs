use core::error;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::event::{self, Event, KeyCode},
    layout::{Constraint, Direction, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use std::{collections::VecDeque, time::Duration};
use tokio::{select, sync::mpsc};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::{agent::Agent, events::DisplayEvent};

pub struct Interface {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    event_rx: mpsc::UnboundedReceiver<DisplayEvent>,
    input: String,
    history: Vec<DisplayEvent>,
    history_scroll_state: ScrollViewState,
    prompt_queue: VecDeque<String>,
}

enum Tick {
    Prompt(String),
    Idle,
    End,
}

impl Interface {
    pub fn new(event_rx: mpsc::UnboundedReceiver<DisplayEvent>) -> Self {
        Self {
            terminal: ratatui::init(),
            event_rx,
            input: String::new(),
            history: Vec::<DisplayEvent>::new(),
            history_scroll_state: ScrollViewState::default(),
            prompt_queue: VecDeque::<String>::new(),
        }
    }
}

impl Interface {
    pub async fn run(&mut self, agent: &mut Agent) -> Result<(), Box<dyn error::Error>> {
        loop {
            let prompt = match self.prompt_queue.pop_front() {
                Some(p) => p,
                None => loop {
                    match self.tick()? {
                        Tick::Prompt(p) => break p,
                        Tick::End => return Ok(()),
                        Tick::Idle => {}
                    }
                },
            };

            self.history.push(DisplayEvent::User(prompt.clone()));
            let agent_future = agent.agent_loop(Some(prompt));
            tokio::pin!(agent_future);

            loop {
                select! {
                    _ = tokio::time::sleep(Duration::from_millis(8)) => {
                        match self.tick()? {
                            Tick::Prompt(p) => self.prompt_queue.push_back(p),
                            Tick::End => return Ok(()),
                            Tick::Idle => {}
                        }
                    }
                    res = &mut agent_future => {
                        match res{
                                Ok(_) => {
                                    break
                                }
                                Err(result) => {
                                    self.history.push(DisplayEvent::Error(result.to_string()));
                                    break
                                }
                            }}
                }
            }
        }
    }

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
                    DisplayEvent::User(chunk) => (
                        format!("❯ {}", chunk),
                        Style::default().bg(Color::Rgb(31, 31, 31)),
                    ),
                    DisplayEvent::Content(chunk) => (chunk.to_string(), Style::default()),
                    DisplayEvent::Reasoning(chunk) => (
                        chunk.to_string(),
                        Style::default().add_modifier(Modifier::ITALIC).dim(),
                    ),
                    DisplayEvent::Error(chunk) => (chunk.to_string(), Style::default().red()),
                    DisplayEvent::ToolCall(tool) => (
                        format!("Tool:{} Arguments:{}", tool.name, tool.arguments),
                        Style::default(),
                    ),
                })
                .collect::<Vec<(String, Style)>>();

            let history_widgets = history_texts
                .iter()
                .map(|(text, style)| {
                    let paragraph = Paragraph::new(tui_markdown::from_str(text))
                        .style(*style)
                        .wrap(Wrap { trim: false })
                        .block(Block::default().padding(Padding::uniform(1)));
                    history_total_height += paragraph.line_count(history_width);
                    paragraph
                })
                .collect::<Vec<Paragraph>>();
            let history_content_size = Size::new(history_width, history_total_height as u16);
            let mut history_scroll_view = ScrollView::new(history_content_size)
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);
            let mut curr_height = 0;
            for item in history_widgets.iter() {
                let item_height = item.line_count(history_width) as u16;
                history_scroll_view.render_widget(
                    item,
                    Rect::new(0, curr_height, history_content_size.width, item_height),
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

    fn tick(&mut self) -> Result<Tick, Box<dyn error::Error>> {
        self.render()?;

        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(&event);
        }

        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Esc => {
                        ratatui::restore();
                        return Ok(Tick::End);
                    }
                    KeyCode::Char(char) => self.input.push(char),
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    KeyCode::Enter => {
                        return Ok(Tick::Prompt(std::mem::take(&mut self.input)));
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
        Ok(Tick::Idle)
    }
}
