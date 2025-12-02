use std::{collections::VecDeque, fmt::Write};

use color_eyre::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{
    action::{Action, DeviceMessage},
    config::Config,
};

const HISTORY_LIMIT: usize = 20;
const MESSAGE_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Editing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageEncoding {
    Utf8,
    Hex,
    Binary,
}

impl MessageEncoding {
    fn label(&self) -> &'static str {
        match self {
            MessageEncoding::Utf8 => "UTF-8",
            MessageEncoding::Hex => "Hex",
            MessageEncoding::Binary => "Binary",
        }
    }
}

#[derive(Debug, Clone)]
struct MessageLine {
    content: DeviceMessage,
    style: Style,
}

impl MessageLine {
    fn new(content: DeviceMessage, style: Style) -> Self {
        Self { content, style }
    }
}

pub struct TerminalScreen {
    action_tx: Option<UnboundedSender<Action>>,
    #[allow(dead_code)]
    config: Option<Config>,
    is_active: bool,
    input_mode: InputMode,
    command_buffer: String,
    command_history: VecDeque<String>,
    incoming_messages: VecDeque<MessageLine>,
    connection_label: Option<String>,
    cursor_index: usize,
    history_position: Option<usize>,
    draft_buffer: Option<String>,
    message_encoding: MessageEncoding,
}

impl Default for InputMode {
    fn default() -> Self {
        InputMode::Normal
    }
}

impl Default for MessageEncoding {
    fn default() -> Self {
        MessageEncoding::Utf8
    }
}

impl Default for TerminalScreen {
    fn default() -> Self {
        Self {
            action_tx: None,
            config: None,
            is_active: false,
            input_mode: InputMode::default(),
            command_buffer: String::new(),
            command_history: VecDeque::new(),
            incoming_messages: VecDeque::new(),
            connection_label: None,
            cursor_index: 0,
            history_position: None,
            draft_buffer: None,
            message_encoding: MessageEncoding::default(),
        }
    }
}

impl TerminalScreen {
    pub fn new() -> Self {
        Self::default()
    }

    fn send(&self, action: Action) -> Result<()> {
        if let Some(tx) = &self.action_tx {
            tx.send(action)?;
        }
        Ok(())
    }

    fn push_history(&mut self, command: String) {
        if command.is_empty() {
            return;
        }
        if self.command_history.len() >= HISTORY_LIMIT {
            self.command_history.pop_front();
        }
        self.command_history.push_back(command);
    }

    fn push_message(&mut self, message: MessageLine) {
        if self.incoming_messages.len() >= MESSAGE_LIMIT {
            self.incoming_messages.pop_front();
        }
        self.incoming_messages.push_back(message);
    }

    fn style_for_message(message: &DeviceMessage) -> Style {
        match message {
            DeviceMessage::Text(text)
                if text.starts_with("Error:") || text.starts_with("Failed to encode command") =>
            {
                Style::default().fg(Color::Red)
            }
            _ => Style::default(),
        }
    }

    fn change_message_encoding(&mut self, encoding: MessageEncoding) -> Result<()> {
        if self.message_encoding != encoding {
            self.message_encoding = encoding;
            self.send(Action::Render)?;
        }
        Ok(())
    }

    fn render_message_text(&self, message: &DeviceMessage) -> String {
        match message {
            DeviceMessage::Text(text) => text.clone(),
            DeviceMessage::Bytes(bytes) => format_bytes(bytes, self.message_encoding),
        }
    }

    fn enter_edit_mode(&mut self) {
        self.input_mode = InputMode::Editing;
        self.cursor_index = self.command_buffer.len();
        self.reset_history_navigation();
    }

    fn reset_history_navigation(&mut self) {
        self.history_position = None;
        self.draft_buffer = None;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_index > 0 {
            if let Some(prev) = self.command_buffer[..self.cursor_index].chars().last() {
                self.cursor_index -= prev.len_utf8();
            } else {
                self.cursor_index = 0;
            }
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_index < self.command_buffer.len() {
            if let Some(next) = self.command_buffer[self.cursor_index..].chars().next() {
                self.cursor_index += next.len_utf8();
            } else {
                self.cursor_index = self.command_buffer.len();
            }
        }
    }

    fn recall_older_command(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        if self.draft_buffer.is_none() {
            self.draft_buffer = Some(self.command_buffer.clone());
        }

        let max_offset = self.command_history.len() - 1;
        let next_offset = match self.history_position {
            Some(offset) if offset >= max_offset => max_offset,
            Some(offset) => offset + 1,
            None => 0,
        };

        if let Some(entry) = self.command_history.iter().rev().nth(next_offset) {
            self.history_position = Some(next_offset);
            self.command_buffer = entry.clone();
            self.cursor_index = self.command_buffer.len();
        }
    }

    fn recall_newer_command(&mut self) {
        if self.command_history.is_empty() {
            return;
        }
        match self.history_position {
            Some(0) => {
                self.history_position = None;
                if let Some(draft) = self.draft_buffer.take() {
                    self.command_buffer = draft;
                } else {
                    self.command_buffer.clear();
                }
                self.cursor_index = self.command_buffer.len();
            }
            Some(offset) => {
                let new_offset = offset.saturating_sub(1);
                if let Some(entry) = self.command_history.iter().rev().nth(new_offset) {
                    self.history_position = Some(new_offset);
                    self.command_buffer = entry.clone();
                    self.cursor_index = self.command_buffer.len();
                } else {
                    self.history_position = None;
                }
            }
            None => {}
        }
    }

    fn handle_editing_key(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Action>> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.input_mode = InputMode::Normal;
                self.reset_history_navigation();
            }
            (KeyCode::Enter, _) => {
                let command = self.command_buffer.clone();
                self.command_buffer.clear();
                self.cursor_index = 0;
                self.input_mode = InputMode::Normal;
                let should_send = !command.trim().is_empty();
                if should_send {
                    self.reset_history_navigation();
                    return Ok(Some(Action::SendCommand(command)));
                }
                self.reset_history_navigation();
            }
            (KeyCode::Backspace, _) => {
                if self.cursor_index > 0 {
                    self.move_cursor_left();
                    if let Some(ch) = self.command_buffer[self.cursor_index..].chars().next() {
                        let len = ch.len_utf8();
                        self.command_buffer
                            .drain(self.cursor_index..self.cursor_index + len);
                    }
                }
            }
            (KeyCode::Char(c), modifiers)
                if modifiers.is_empty() || modifiers == KeyModifiers::SHIFT =>
            {
                self.command_buffer.insert(self.cursor_index, c);
                self.cursor_index += c.len_utf8();
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.command_buffer.clear();
                self.cursor_index = 0;
                self.reset_history_navigation();
            }
            (KeyCode::Left, KeyModifiers::NONE) => self.move_cursor_left(),
            (KeyCode::Right, KeyModifiers::NONE) => self.move_cursor_right(),
            (KeyCode::Home, _) => self.cursor_index = 0,
            (KeyCode::End, _) => self.cursor_index = self.command_buffer.len(),
            (KeyCode::Delete, _) => {
                if self.cursor_index < self.command_buffer.len() {
                    if let Some(ch) = self.command_buffer[self.cursor_index..].chars().next() {
                        let len = ch.len_utf8();
                        self.command_buffer
                            .drain(self.cursor_index..self.cursor_index + len);
                    }
                }
            }
            (KeyCode::Up, KeyModifiers::NONE) => {
                self.recall_older_command();
            }
            (KeyCode::Down, KeyModifiers::NONE) => {
                self.recall_newer_command();
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_normal_key(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Action>> {
        use crossterm::event::{KeyCode, KeyModifiers};

        match (key.code, key.modifiers) {
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                return Ok(Some(Action::ToggleHelp));
            }
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                self.enter_edit_mode();
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.change_message_encoding(MessageEncoding::Utf8)?;
            }
            (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                self.change_message_encoding(MessageEncoding::Hex)?;
            }
            (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.change_message_encoding(MessageEncoding::Binary)?;
            }
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.send(Action::Quit)?;
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL)
            | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.send(Action::Quit)?;
            }
            _ => {}
        }
        Ok(None)
    }
}

impl Component for TerminalScreen {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.action_tx = Some(tx);
        Ok(())
    }

    fn register_config_handler(&mut self, config: Config) -> Result<()> {
        self.config = Some(config);
        Ok(())
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Action>> {
        if !self.is_active {
            return Ok(None);
        }

        match self.input_mode {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Editing => self.handle_editing_key(key),
        }
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ShowMain => {
                self.is_active = true;
                self.input_mode = InputMode::Normal;
                self.cursor_index = self.command_buffer.len();
                self.reset_history_navigation();
            }
            Action::ShowPreconnect | Action::ShowConnecting | Action::ShowError(_) => {
                self.is_active = false;
                self.input_mode = InputMode::Normal;
                self.cursor_index = self.command_buffer.len();
                self.reset_history_navigation();
            }
            Action::CommandSent(command) => {
                self.push_history(command);
                self.command_buffer.clear();
                self.cursor_index = 0;
                self.reset_history_navigation();
            }
            Action::IncomingMessage(message) => {
                let style = Self::style_for_message(&message);
                self.push_message(MessageLine::new(message, style));
            }
            Action::ConnectionEstablished { port, baud_rate } => {
                self.connection_label = Some(format!("{port} @ {baud_rate} baud"));
            }
            _ => {}
        }
        Ok(None)
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> Result<()> {
        if !self.is_active {
            return Ok(());
        }

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(4),
                    Constraint::Length(3),
                    Constraint::Length(6),
                    Constraint::Min(10),
                ]
                .as_ref(),
            )
            .split(area);

        let connection_line = self
            .connection_label
            .clone()
            .unwrap_or_else(|| "Not connected".into());
        let mode_label = match self.input_mode {
            InputMode::Normal => "Normal",
            InputMode::Editing => "Editing",
        };
        let instruction = vec![
            Line::from(format!(
                "Connected: {connection_line} • Mode: {mode_label} • View: {}",
                self.message_encoding.label()
            )),
            Line::from(
                "Press e to edit the command, Enter to send, Esc to cancel editing, q to quit.",
            ),
            Line::from("Ctrl+u UTF-8, Ctrl+h Hex, Ctrl+b Binary to change message view."),
        ];
        frame.render_widget(
            Paragraph::new(instruction)
                .block(Block::default().title("Session").borders(Borders::ALL)),
            layout[0],
        );

        let command_line = if self.input_mode == InputMode::Editing {
            let cursor_index = self.cursor_index.min(self.command_buffer.len());
            let (left, right) = self.command_buffer.split_at(cursor_index);
            Line::from(vec![
                Span::styled("Command> ", Style::default().fg(Color::Cyan)),
                Span::raw(left.to_string()),
                Span::styled("┃", Style::default().fg(Color::Yellow)),
                Span::raw(right.to_string()),
            ])
        } else {
            Line::from(vec![
                Span::styled("Command> ", Style::default().fg(Color::Cyan)),
                Span::raw(self.command_buffer.clone()),
            ])
        };
        frame.render_widget(
            Paragraph::new(Text::from(command_line)).block(
                Block::default()
                    .title("Command Input")
                    .borders(Borders::ALL),
            ),
            layout[1],
        );

        let history_items: Vec<ListItem> = self
            .command_history
            .iter()
            .rev()
            .map(|entry| ListItem::new(entry.clone()))
            .collect();
        frame.render_widget(
            List::new(history_items).block(
                Block::default()
                    .title("Command History")
                    .borders(Borders::ALL),
            ),
            layout[2],
        );

        let bottom_cat = Span::styled(
            " ᓚᘏᗢ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );

        let message_block = Block::default()
            .title(format!(
                "Device Messages ({})",
                self.message_encoding.label()
            ))
            .title_bottom(bottom_cat)
            .borders(Borders::ALL);

        let message_area = message_block.inner(layout[3]);
        let available_width = message_area.width as usize;

        let mut message_items: Vec<ListItem> = self
            .incoming_messages
            .iter()
            .rev()
            .map(|msg| {
                let formatted = self.render_message_text(&msg.content);
                let rendered = if available_width == 0 {
                    formatted
                } else {
                    let truncated: String = formatted.chars().take(available_width).collect();
                    format!("{:<width$}", truncated, width = available_width)
                };

                ListItem::new(Line::from(vec![Span::styled(rendered, msg.style)]))
            })
            .collect();

        if message_items.is_empty() {
            message_items.push(ListItem::new(Line::from("No messages received yet.")));
        }

        frame.render_widget(Clear, layout[3]);
        frame.render_widget(List::new(message_items).block(message_block), layout[3]);

        Ok(())
    }
}

fn format_bytes(bytes: &[u8], encoding: MessageEncoding) -> String {
    match encoding {
        MessageEncoding::Utf8 => {
            if bytes.is_empty() {
                "<empty>".into()
            } else {
                String::from_utf8_lossy(bytes).into()
            }
        }
        MessageEncoding::Hex => format_hex(bytes),
        MessageEncoding::Binary => format_binary(bytes),
    }
}

fn format_hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "<empty>".into();
    }

    let mut output = String::with_capacity(bytes.len() * 3 + 2);
    output.push_str("0x");
    for (idx, byte) in bytes.iter().enumerate() {
        if idx > 0 {
            output.push(' ');
        }
        let _ = write!(&mut output, "{:02X}", byte);
    }
    output
}

fn format_binary(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "<empty>".into();
    }

    let mut output = String::with_capacity(bytes.len() * 11);
    for (idx, byte) in bytes.iter().enumerate() {
        if idx > 0 {
            output.push(' ');
        }
        output.push_str("0b");
        let _ = write!(&mut output, "{:08b}", byte);
    }
    output
}
