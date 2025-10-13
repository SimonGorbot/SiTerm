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
pub struct ErrorScreen {
    action_tx: Option<UnboundedSender<Action>>,
    #[allow(dead_code)]
    config: Option<Config>,
    is_active: bool,
    message: Option<String>,
}

impl ErrorScreen {
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

impl Component for ErrorScreen {
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
            (KeyCode::Enter, _) | (KeyCode::Esc, _) | (KeyCode::Char('r'), KeyModifiers::NONE) => {
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
            Action::ShowError(message) => {
                self.is_active = true;
                self.message = Some(message);
            }
            Action::ShowPreconnect => {
                self.is_active = false;
                self.message = None;
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
            "Connection Error",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );

        let body = self
            .message
            .clone()
            .unwrap_or_else(|| "Unknown error.".into());

        frame.render_widget(
            Paragraph::new(body).block(Block::default().title(title).borders(Borders::ALL)),
            layout[0],
        );

        let instructions = vec![
            Line::from(
                "The connected device did not respond with the expected SiTerm firmware signature.",
            ),
            Line::from("Press Enter to return to the preconnect screen, or q to quit."),
        ];
        frame.render_widget(
            Paragraph::new(instructions)
                .block(Block::default().title("Next steps").borders(Borders::ALL)),
            layout[1],
        );

        Ok(())
    }
}

/*
           ／＞　 フ
          | 　_　_|
        ／` ミ＿xノ
       /　　　　 |
      /　 ヽ　　 ﾉ
     │　　|　|　|
 ／￣|　　 |　|　|
(￣ヽ＿_ヽ_)__)
＼二)
*/
