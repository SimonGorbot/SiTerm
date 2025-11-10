use std::cmp::min;

use color_eyre::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Ports,
    Baud,
}

pub struct PreconnectScreen {
    action_tx: Option<UnboundedSender<Action>>,
    #[allow(dead_code)]
    config: Option<Config>,
    focus: Focus,
    is_active: bool,
    ports: Vec<String>,
    baud_rates: Vec<u32>,
    port_index: usize,
    baud_index: usize,
    status_message: Option<String>,
}

impl Default for PreconnectScreen {
    fn default() -> Self {
        Self {
            action_tx: None,
            config: None,
            focus: Focus::Ports,
            is_active: true,
            ports: Vec::new(),
            baud_rates: vec![9_600, 19_200, 38_400, 57_600, 115_200],
            port_index: 0,
            baud_index: 0,
            status_message: None,
        }
    }
}

impl PreconnectScreen {
    pub fn new() -> Self {
        Self::default()
    }

    fn send(&self, action: Action) -> Result<()> {
        if let Some(tx) = &self.action_tx {
            tx.send(action)?;
        }
        Ok(())
    }

    fn select_next_port(&mut self) {
        if self.ports.is_empty() {
            return;
        }
        self.port_index = min(self.port_index + 1, self.ports.len() - 1);
    }

    fn select_previous_port(&mut self) {
        if self.ports.is_empty() {
            return;
        }
        if self.port_index > 0 {
            self.port_index -= 1;
        }
    }

    fn select_next_baud(&mut self) {
        if self.baud_rates.is_empty() {
            return;
        }
        self.baud_index = min(self.baud_index + 1, self.baud_rates.len() - 1);
    }

    fn select_previous_baud(&mut self) {
        if self.baud_rates.is_empty() {
            return;
        }
        if self.baud_index > 0 {
            self.baud_index -= 1;
        }
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Ports => Focus::Baud,
            Focus::Baud => Focus::Ports,
        };
    }

    fn attempt_connect(&mut self) -> Result<Option<Action>> {
        if self.ports.is_empty() {
            self.status_message = Some("No serial ports detected. Press r to refresh.".into());
            return Ok(None);
        }
        let port = self.ports[self.port_index].clone();
        let baud_rate = self.baud_rates[self.baud_index];
        Ok(Some(Action::Connect { port, baud_rate }))
    }
}

impl Component for PreconnectScreen {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.action_tx = Some(tx);
        Ok(())
    }

    fn register_config_handler(&mut self, config: Config) -> Result<()> {
        self.config = Some(config);
        Ok(())
    }

    fn init(&mut self, _area: ratatui::layout::Size) -> Result<()> {
        self.send(Action::RefreshPorts)?;
        Ok(())
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Action>> {
        use crossterm::event::{KeyCode, KeyModifiers};

        if !self.is_active {
            return Ok(None);
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.send(Action::Quit)?;
            }
            (KeyCode::Char('h'), KeyModifiers::NONE) => {
                return Ok(Some(Action::ToggleHelp));
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                self.status_message = Some("Refreshing ports...".into());
                self.send(Action::RefreshPorts)?;
            }
            (KeyCode::Tab, _)
            | (KeyCode::Left, KeyModifiers::NONE)
            | (KeyCode::Right, KeyModifiers::NONE) => {
                self.toggle_focus();
            }
            (KeyCode::Up, KeyModifiers::NONE) => match self.focus {
                Focus::Ports => self.select_previous_port(),
                Focus::Baud => self.select_previous_baud(),
            },
            (KeyCode::Down, KeyModifiers::NONE) => match self.focus {
                Focus::Ports => self.select_next_port(),
                Focus::Baud => self.select_next_baud(),
            },
            (KeyCode::Enter, _) => return self.attempt_connect(),
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ShowPreconnect => {
                self.is_active = true;
                self.status_message = None;
            }
            Action::ShowConnecting | Action::ShowMain | Action::ShowError(_) => {
                self.is_active = false;
            }
            Action::PortsUpdated(ports) => {
                self.ports = ports;
                if self.port_index >= self.ports.len() {
                    self.port_index = self.ports.len().saturating_sub(1);
                }
                if self.ports.is_empty() {
                    self.port_index = 0;
                    self.status_message = Some(
                        "No serial ports detected. Connect a device and press r to refresh.".into(),
                    );
                } else {
                    self.status_message = Some(" Select a port and press Enter to connect.".into());
                }
            }
            Action::ConnectionFailed(message) => {
                self.status_message = Some(message);
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
                    Constraint::Length(7),
                    Constraint::Length(4),
                    Constraint::Min(7),
                    Constraint::Length(1),
                ]
                .as_ref(),
            )
            .split(area);

        let welcome_text = vec![
            Line::from("Welcome to ").style(Modifier::ITALIC),
            Line::from(r"   _____ _ ______                  ").style(Color::Cyan),
            Line::from(r"  / ___/(_)_  __/__  _________ ___ ").style(Color::Cyan),
            Line::from(r"  \__ \/ / / / / _ \/ ___/ __ `__ \").style(Color::LightCyan),
            Line::from(r" ___/ / / / / /  __/ /  / / / / / /").style(Color::LightBlue),
            Line::from(r"/____/_/ /_/  \___/_/  /_/ /_/ /_/").style(Color::Blue),
        ];
        frame.render_widget(Paragraph::new(welcome_text), layout[0]);

        let instruction_lines = vec![
            Line::from("Select a serial port and baud rate to connect."),
            Line::from(
                "Use ↑/↓ to navigate, Tab to switch lists, Enter to connect, r to refresh, q to quit.",
            ),
        ];
        frame.render_widget(
            Paragraph::new(instruction_lines)
                .block(Block::default().title("Instructions").borders(Borders::ALL)),
            layout[1],
        );

        let lists = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(65), Constraint::Percentage(35)].as_ref())
            .split(layout[2]);

        let cat_spot = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(5), Constraint::Max(7)].as_ref())
            .split(lists[1]);

        #[rustfmt::skip]
        let cat_ascii = vec![
            Line::from(""),
            Line::from(vec![Span::styled(" ╱|、    ", Style::default().fg(Color::Cyan))]).right_aligned(),
            Line::from(vec![Span::styled("(˚ˎ。7   ", Style::default().fg(Color::LightCyan))]).right_aligned(),
            Line::from(vec![Span::styled("|、˜〵   ", Style::default().fg(Color::LightBlue))]).right_aligned(),
            Line::from(vec![Span::styled("じしˍ,)ノ", Style::default().fg(Color::Blue))]).right_aligned(),
        ];

        frame.render_widget(
            Paragraph::new(cat_ascii).block(Block::bordered().title("Your new friend")),
            cat_spot[1],
        );

        let highlight_style = Style::default()
            .fg(Color::Black)
            .bg(Color::LightBlue)
            .add_modifier(Modifier::BOLD);

        let port_block_style = if self.focus == Focus::Ports {
            Style::default().fg(Color::Blue)
        } else {
            Style::default()
        };
        let baud_block_style = if self.focus == Focus::Baud {
            Style::default().fg(Color::Blue)
        } else {
            Style::default()
        };

        let port_items: Vec<ListItem> = if self.ports.is_empty() {
            vec![ListItem::new(Span::raw("No ports detected."))]
        } else {
            self.ports
                .iter()
                .map(|p| ListItem::new(Span::raw(p.clone())))
                .collect()
        };
        let mut ports_state = ListState::default();
        if !self.ports.is_empty() {
            ports_state.select(Some(self.port_index));
        }
        frame.render_stateful_widget(
            List::new(port_items)
                .block(
                    Block::default()
                        .title(Span::styled("Serial Ports", port_block_style))
                        .borders(Borders::ALL),
                )
                .highlight_style(highlight_style)
                .highlight_symbol("➤ "),
            lists[0],
            &mut ports_state,
        );

        let baud_items: Vec<ListItem> = self
            .baud_rates
            .iter()
            .map(|b| ListItem::new(Span::raw(format!("{b}"))))
            .collect();
        let mut baud_state = ListState::default();
        baud_state.select(Some(self.baud_index));
        frame.render_stateful_widget(
            List::new(baud_items)
                .block(
                    Block::default()
                        .title(Span::styled("Baud Rates", baud_block_style))
                        .borders(Borders::ALL),
                )
                .highlight_style(highlight_style)
                .highlight_symbol("➤ "),
            cat_spot[0],
            &mut baud_state,
        );

        let status = self
            .status_message
            .clone()
            .unwrap_or_else(|| "Ready.".into());
        frame.render_widget(Paragraph::new(status), layout[3]);

        Ok(())
    }
}
