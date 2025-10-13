use color_eyre::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use tokio::sync::mpsc::UnboundedSender;

use super::Component;
use crate::{action::Action, config::Config};

#[derive(Default)]
pub struct ConnectingScreen {
    action_tx: Option<UnboundedSender<Action>>,
    #[allow(dead_code)]
    config: Option<Config>,
    is_active: bool,
    port: Option<String>,
    baud_rate: Option<u32>,
}

impl ConnectingScreen {
    pub fn new() -> Self {
        Self::default()
    }

    fn send(&self, action: Action) -> Result<()> {
        if let Some(tx) = &self.action_tx {
            tx.send(action)?;
        }
        Ok(())
    }
}

impl Component for ConnectingScreen {
    fn register_action_handler(&mut self, tx: UnboundedSender<Action>) -> Result<()> {
        self.action_tx = Some(tx);
        Ok(())
    }

    fn register_config_handler(&mut self, config: Config) -> Result<()> {
        self.config = Some(config);
        Ok(())
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> Result<Option<Action>> {
        use crossterm::event::{KeyCode, KeyModifiers};

        if !self.is_active {
            return Ok(None);
        }

        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.send(Action::ShowPreconnect)?;
            }
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                self.send(Action::Quit)?;
            }
            _ => {}
        }
        Ok(None)
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ShowConnecting => {
                self.is_active = true;
            }
            Action::Connect { port, baud_rate } => {
                self.port = Some(port);
                self.baud_rate = Some(baud_rate);
            }
            Action::ConnectionEstablished { .. }
            | Action::ConnectionFailed(_)
            | Action::ShowPreconnect
            | Action::ShowMain
            | Action::ShowError(_) => {
                self.is_active = false;
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
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)].as_ref())
            .split(area);

        let title = Span::styled(
            "Connecting to device…",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        );
        let mut lines = vec![
            Line::from("Testing serial connection and verifying SiTerm firmware."),
            Line::from("Press Esc to cancel and return to selection."),
        ];

        if let (Some(port), Some(baud)) = (&self.port, self.baud_rate) {
            lines.push(Line::from(format!("Port: {port}")));
            lines.push(Line::from(format!("Baud rate: {baud}")));
        }

        frame.render_widget(
            Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL)),
            layout[0],
        );

        frame.render_widget(
            Paragraph::new("Waiting for handshake response…")
                .block(Block::default().title("Status").borders(Borders::ALL)),
            layout[1],
        );

        Ok(())
    }
}
