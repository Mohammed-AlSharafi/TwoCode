use core::error;
use crossterm::{
    event::{EnableMouseCapture, MouseButton, MouseEventKind},
    execute,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::event::{self, Event, KeyCode, KeyModifiers},
    layout::{Constraint, Direction, Layout, Position, Rect, Size},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use std::{collections::VecDeque, io::stdout, time::Duration};
use tokio::{select, sync::mpsc};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::{agent::Agent, events::DisplayEvent};

const SCROLL_SPEED: u8 = 2;

pub struct Interface {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    event_rx: mpsc::UnboundedReceiver<DisplayEvent>,
    input: String,
    history: Vec<DisplayBlock>,
    history_scroll_state: ScrollViewState,
    prompt_queue: VecDeque<String>,
}

enum Tick {
    Prompt(String),
    Idle,
    End,
    Kill,
}
enum BlockType {
    User,
    Reasoning { expanded: bool },
    Content,
    ToolCall,
    Error,
}
struct DisplayBlock {
    block_type: BlockType,
    content: String,
    area: Rect,
}

impl DisplayBlock {
    fn new(block_type: BlockType, content: String) -> Self {
        DisplayBlock {
            block_type,
            content,
            area: Rect::default(),
        }
    }

    fn update_area(&mut self, new_area: Rect) {
        let _ = self.area = new_area;
    }
}

impl Interface {
    pub fn new(event_rx: mpsc::UnboundedReceiver<DisplayEvent>) -> Self {
        let _ = execute!(stdout(), EnableMouseCapture);

        Self {
            terminal: ratatui::init(),
            event_rx,
            input: String::new(),
            history: Vec::<DisplayBlock>::new(),
            history_scroll_state: ScrollViewState::default(),
            prompt_queue: VecDeque::<String>::new(),
        }
    }

    pub async fn run(&mut self, agent: &mut Agent) -> Result<(), Box<dyn error::Error>> {
        loop {
            //first loop to render and execute prompt
            let prompt = match self.prompt_queue.pop_front() {
                Some(p) => p,
                None => loop {
                    tokio::time::sleep(Duration::from_millis(8)).await;
                    match self.tick()? {
                        Tick::Prompt(p) => break p,
                        Tick::Kill => {
                            self.terminate_interface();
                            return Ok(());
                        }
                        _ => {}
                    }
                },
            };

            self.history
                .push(DisplayBlock::new(BlockType::User, prompt.to_owned()));
            self.history_scroll_state.scroll_to_bottom();

            let agent_future = agent.agent_loop(Some(prompt));
            tokio::pin!(agent_future);

            //second loop while to render and queue prompts while executing prompt
            loop {
                select! {
                    _ = tokio::time::sleep(Duration::from_millis(8)) => {
                        match self.tick()? {
                            Tick::Prompt(p) => self.prompt_queue.push_back(p),
                            Tick::End => break,
                            Tick::Kill => break,
                            Tick::Idle => {}
                        }
                    }
                    res = &mut agent_future => {
                        match res{
                            Ok(_) => {
                                break
                            }
                            Err(result) => {
                                self.history.push(DisplayBlock::new(BlockType::Error, result.to_string()));
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
                .block(Block::default().borders(Borders::ALL).title("Prompt")).wrap(Wrap { trim: false });
            frame.render_widget(input_widget, prompt_chunk);

            let history_width = history_chunk.width.max(1); //we pick the max between 1 and history chunk's width
            let history_texts = self
                .history
                .iter()
                .map(|item| match item.block_type {
                    BlockType::User => (
                        format!("❯ {}", item.content.to_owned()),
                        Style::default().bg(Color::Rgb(31, 31, 31)),
                    ),
                    BlockType::Content => (item.content.to_owned(), Style::default()),
                    BlockType::Reasoning { expanded } => (
                        if expanded {
                            item.content.to_owned()
                        } else {
                            if item.content.len() > 0 {
                                "Thinking >".to_owned()
                            } else {
                                "Thinking".to_owned()
                            }
                        },
                        Style::default().add_modifier(Modifier::ITALIC).dim(),
                    ),
                    BlockType::Error => (item.content.to_owned(), Style::default().red()),
                    BlockType::ToolCall => (item.content.to_owned(), Style::default()),
                })
                .collect::<Vec<(String, Style)>>();

            let mut history_total_height = 0;
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

            //create scrollview of the whole history
            let history_content_size = Size::new(history_width, history_total_height as u16);
            let mut history_scroll_view = ScrollView::new(history_content_size)
                .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);

            let mut curr_height = 0;
            for (item, block) in history_widgets.iter().zip(self.history.iter_mut()) {
                let item_height = item.line_count(history_width) as u16;
                let rect = Rect::new(0, curr_height, history_content_size.width, item_height);
                block.update_area(rect);
                history_scroll_view.render_widget(item, rect);
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
            //see of the current chunk is a continuation for the previous chunk
            match (event, &last.block_type) {
                | (DisplayEvent::Content(new), BlockType::Content)
                | (DisplayEvent::Reasoning(new), BlockType::Reasoning { .. })
                | (DisplayEvent::Error(new), BlockType::Error) => {
                    last.content.push_str(new);
                    self.history_scroll_state.scroll_to_bottom();
                    return;
                }
                _ => {}
            }
        }

        //only reached if this is a new chunk type
        let (block_type, content) = match event {
            DisplayEvent::Content(c) => (BlockType::Content, c.to_owned()),
            DisplayEvent::Error(c) => (BlockType::Error, c.to_owned()),
            DisplayEvent::Reasoning(c) => (BlockType::Reasoning { expanded: false }, c.to_owned()),
            DisplayEvent::ToolCall(c) => (
                BlockType::ToolCall,
                format!("Tool:{} {}", c.name, c.arguments),
            ),
        };

        self.history.push(DisplayBlock {
            block_type,
            content,
            area: Rect::default(),
        });
        self.history_scroll_state.scroll_to_bottom();
    }

    fn tick(&mut self) -> Result<Tick, Box<dyn error::Error>> {
        self.render()?;

        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(&event);
        }

        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(Tick::Kill);
                    }
                    (KeyCode::Esc, _) => {
                        return Ok(Tick::End);
                    }
                    (KeyCode::Char(char), _) => self.input.push(char),
                    (KeyCode::Backspace, _) => {
                        self.input.pop();
                    }
                    (KeyCode::Enter, _) => {
                        return Ok(Tick::Prompt(std::mem::take(&mut self.input)));
                    }
                    (KeyCode::Up, _) => {
                        self.history_scroll_state.scroll_up();
                    }
                    (KeyCode::Down, _) => {
                        self.history_scroll_state.scroll_down();
                    }
                    _ => {}
                }
            } else if let Event::Mouse(mouse_event) = event::read()? {
                match mouse_event.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        // Convert screen coordinates to content coordinates by
                        // adding the scroll offset, so the click lands on the
                        // correct block regardless of scroll position.
                        let scroll_offset = self.history_scroll_state.offset();
                        let content_pos = Position::new(
                            mouse_event.column.saturating_add(scroll_offset.x),
                            mouse_event.row.saturating_add(scroll_offset.y),
                        );

                        for block in &mut self.history {
                            if block.area.contains(content_pos) {
                                match &mut block.block_type {
                                    BlockType::Reasoning { expanded } => *expanded = !*expanded,
                                    _ => {}
                                }
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        for _ in 0..SCROLL_SPEED {
                            self.history_scroll_state.scroll_down();
                        }
                    }

                    MouseEventKind::ScrollUp => {
                        for _ in 0..SCROLL_SPEED {
                            self.history_scroll_state.scroll_up();
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Tick::Idle)
    }

    fn terminate_interface(&self) {
        ratatui::restore();
    }
}
