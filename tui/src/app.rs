use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::Rect;
use serde::{Deserialize, Serialize};
use std::{fmt::Write, str};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};
use tracing::debug;

use crate::{
    action::Action,
    components::{
        Component, connecting::ConnectingScreen, error_view::ErrorScreen,
        preconnect::PreconnectScreen, terminal::TerminalScreen,
    },
    tui::{Event, Tui},
};

use protocol_host::{
    EncodeError, TransportCodecError, HANDSHAKE_COMMAND, HANDSHAKE_DELIMITER, HANDSHAKE_RESPONSE,
    HANDSHAKE_TIMEOUT, encode_command, encode_transport_frame, try_decode_transport_frame,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    Preconnect,
    Connecting,
    Main,
    Error,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Preconnect
    }
}

pub struct App {
    tick_rate: f64,
    frame_rate: f64,
    components: Vec<Box<dyn Component>>,
    should_quit: bool,
    should_suspend: bool,
    mode: Mode,
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,
    serial_tx: Option<mpsc::UnboundedSender<String>>,
}

impl App {
    pub fn new(tick_rate: f64, frame_rate: f64) -> Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        Ok(Self {
            tick_rate,
            frame_rate,
            components: vec![
                Box::new(PreconnectScreen::new()),
                Box::new(ConnectingScreen::new()),
                Box::new(ErrorScreen::new()),
                Box::new(TerminalScreen::new()),
            ],
            should_quit: false,
            should_suspend: false,
            mode: Mode::Preconnect,
            action_tx,
            action_rx,
            serial_tx: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut tui = Tui::new()?
            // .mouse(true) // uncomment this line to enable mouse support
            .tick_rate(self.tick_rate)
            .frame_rate(self.frame_rate);
        tui.enter()?;

        for component in self.components.iter_mut() {
            component.register_action_handler(self.action_tx.clone())?;
        }
        for component in self.components.iter_mut() {
            component.init(tui.size()?)?;
        }

        let action_tx = self.action_tx.clone();
        loop {
            tokio::select! {
                maybe_event = tui.next_event() => {
                    match maybe_event {
                        Some(event) => {
                            self.handle_event(event)?;
                            self.drain_pending_actions(&mut tui)?;
                        }
                        None => break,
                    }
                }
                maybe_action = self.action_rx.recv() => {
                    match maybe_action {
                        Some(action) => {
                            self.handle_action(&mut tui, action)?;
                            self.drain_pending_actions(&mut tui)?;
                        }
                        None => break,
                    }
                }
            }

            if self.should_suspend {
                tui.suspend()?;
                action_tx.send(Action::Resume)?;
                action_tx.send(Action::ClearScreen)?;
                tui.enter()?;
            } else if self.should_quit {
                tui.stop()?;
                break;
            }
        }
        tui.exit()?;
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> Result<()> {
        let action_tx = self.action_tx.clone();
        match event {
            Event::Quit => action_tx.send(Action::Quit)?,
            Event::Tick => action_tx.send(Action::Tick)?,
            Event::Render => action_tx.send(Action::Render)?,
            Event::Resize(x, y) => action_tx.send(Action::Resize(x, y))?,
            Event::Key(key) => self.handle_key_event(key)?,
            _ => {}
        }
        for component in self.components.iter_mut() {
            if let Some(action) = component.handle_events(Some(event.clone()))? {
                action_tx.send(action)?;
            }
        }
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        let action_tx = self.action_tx.clone();
        if Self::is_ctrl_key(&key, 'c') || Self::is_ctrl_key(&key, 'd') {
            action_tx.send(Action::Quit)?;
        }
        Ok(())
    }

    fn handle_action(&mut self, tui: &mut Tui, action: Action) -> Result<()> {
        if !matches!(action, Action::Tick | Action::Render) {
            debug!("{action:?}");
        }
        let action_clone = action.clone();
        match action_clone {
            Action::Tick => {}
            Action::Quit => self.should_quit = true,
            Action::Suspend => self.should_suspend = true,
            Action::Resume => self.should_suspend = false,
            Action::ClearScreen => tui.terminal.clear()?,
            Action::Resize(w, h) => self.handle_resize(tui, w, h)?,
            Action::Render => self.render(tui)?,
            Action::ShowPreconnect => {
                self.mode = Mode::Preconnect;
                self.serial_tx = None;
                self.action_tx.send(Action::Render)?;
            }
            Action::ShowConnecting => {
                self.mode = Mode::Connecting;
                self.action_tx.send(Action::Render)?;
            }
            Action::ShowMain => {
                self.mode = Mode::Main;
                self.action_tx.send(Action::Render)?;
            }
            Action::ShowError(_) => {
                self.mode = Mode::Error;
                self.action_tx.send(Action::Render)?;
            }
            Action::RefreshPorts => {
                let ports = tokio_serial::available_ports()
                    .map(|v| v.iter().map(|p| p.port_name.clone()).collect())?;
                self.action_tx.send(Action::PortsUpdated(ports))?;
            }
            Action::PortsUpdated(_) => {}
            Action::Connect { port, baud_rate } => {
                self.mode = Mode::Connecting;
                self.action_tx.send(Action::ShowConnecting)?;
                self.serial_tx = None;
                self.spawn_connection_task(port, baud_rate);
            }
            Action::ConnectionEstablished { port, baud_rate } => {
                self.action_tx.send(Action::ShowMain)?;
                self.action_tx.send(Action::IncomingMessage(format!(
                    "Connected to {port} @ {baud_rate} baud"
                )))?;
            }
            Action::ConnectionFailed(message) => {
                self.serial_tx = None;
                self.action_tx.send(Action::ShowError(message.clone()))?;
            }
            Action::SendCommand(command) => match &self.serial_tx {
                Some(tx) => match tx.send(command.clone()) {
                    Ok(_) => {
                        self.action_tx.send(Action::CommandSent(command))?;
                    }
                    Err(_) => {
                        self.serial_tx = None;
                        self.action_tx.send(Action::ConnectionFailed(
                            "Serial writer is unavailable.".into(),
                        ))?;
                    }
                },
                None => {
                    self.action_tx.send(Action::ConnectionFailed(
                        "Serial connection is not ready.".into(),
                    ))?;
                }
            },
            Action::CommandSent(_) => {}
            Action::IncomingMessage(_) => {}
            Action::Error(_) => {}
        }
        for component in self.components.iter_mut() {
            if let Some(next_action) = component.update(action.clone())? {
                self.action_tx.send(next_action)?;
            }
        }
        Ok(())
    }

    fn drain_pending_actions(&mut self, tui: &mut Tui) -> Result<()> {
        while let Ok(action) = self.action_rx.try_recv() {
            self.handle_action(tui, action)?;
        }
        Ok(())
    }

    fn handle_resize(&mut self, tui: &mut Tui, w: u16, h: u16) -> Result<()> {
        tui.resize(Rect::new(0, 0, w, h))?;
        self.render(tui)?;
        Ok(())
    }

    fn render(&mut self, tui: &mut Tui) -> Result<()> {
        tui.draw(|frame| {
            for component in self.components.iter_mut() {
                if let Err(err) = component.draw(frame, frame.area()) {
                    let _ = self
                        .action_tx
                        .send(Action::Error(format!("Failed to draw: {:?}", err)));
                }
            }
        })?;
        Ok(())
    }

    fn is_ctrl_key(key: &KeyEvent, chr: char) -> bool {
        matches!(
            (key.code, key.modifiers),
            (KeyCode::Char(c), KeyModifiers::CONTROL) if c == chr
        )
    }
}

impl App {
    // TODO: Implement timeouts for all steps in connection process.
    fn spawn_connection_task(&mut self, port: String, baud_rate: u32) {
        let (serial_tx, serial_rx) = mpsc::unbounded_channel::<String>();
        self.serial_tx = Some(serial_tx);
        let action_tx = self.action_tx.clone();
        tokio::spawn(async move {
            match App::establish_serial_stream(&port, baud_rate).await {
                Ok(serial_stream) => {
                    let _ = action_tx.send(Action::ConnectionEstablished {
                        port: port.clone(),
                        baud_rate,
                    });
                    let _ = action_tx.send(Action::ShowMain);
                    App::run_serial_session(serial_stream, serial_rx, action_tx.clone()).await;
                }
                Err(message) => {
                    let _ = action_tx.send(Action::ConnectionFailed(message));
                }
            }
        });
    }

    async fn establish_serial_stream(port: &str, baud_rate: u32) -> Result<SerialStream, String> {
        let serial_port_builder = tokio_serial::new(port, baud_rate)
            .data_bits(tokio_serial::DataBits::Eight)
            .stop_bits(tokio_serial::StopBits::One)
            .parity(tokio_serial::Parity::None)
            .timeout(std::time::Duration::from_millis(1000));

        let mut serial_port = serial_port_builder
            .open_native_async()
            .map_err(|e| format!("Failed to open serial port {port}.\nError: {e}"))?;

        serial_port
            .clear(tokio_serial::ClearBuffer::All)
            .map_err(|e| format!("Failed to clear serial port buffer.\nError {e}"))?;

        serial_port
            .write_all((HANDSHAKE_COMMAND.to_owned() + HANDSHAKE_DELIMITER).as_bytes())
            .await
            .map_err(|e| {
                format!("Failed to write handshake command using serial port.\nError {e}")
            })?;

        let mut handshake_buffer = [0u8; HANDSHAKE_RESPONSE.len()];
        let read_result = timeout(
            HANDSHAKE_TIMEOUT,
            serial_port.read_exact(&mut handshake_buffer),
        )
        .await;

        let handshake_bytes = match read_result {
            Err(_) => {
                return Err("Timed out waiting for handshake response.".into());
            }
            Ok(Err(e)) => {
                return Err(format!("Handshake read failed: {e}"));
            }
            Ok(Ok(_)) => handshake_buffer,
        };

        let response_as_string = str::from_utf8(&handshake_bytes)
            .map_err(|e| format!("Handshake conversion to str failed: {e}"))?;

        if response_as_string != HANDSHAKE_RESPONSE {
            return Err(format!(
                "Invalid handshake response received.\n Response received: {response_as_string}"
            ));
        }

        Ok(serial_port)
    }

    async fn run_serial_session(
        serial_stream: SerialStream,
        serial_rx: mpsc::UnboundedReceiver<String>,
        action_tx: mpsc::UnboundedSender<Action>,
    ) {
        let (reader_half, writer_half) = tokio::io::split(serial_stream);

        let writer_action_tx = action_tx.clone();
        let writer_task = tokio::spawn(async move {
            let mut writer_half = writer_half;
            let mut command_rx = serial_rx;
            while let Some(command) = command_rx.recv().await {
                let trimmed = command.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match encode_command(trimmed) {
                    Ok(payload) => match encode_transport_frame(&payload) {
                        Ok(frame) => {
                            if let Err(e) = writer_half.write_all(&frame).await {
                                let _ = writer_action_tx.send(Action::ConnectionFailed(format!(
                                    "Serial write failed: {e}"
                                )));
                                break;
                            }
                        }
                        Err(err) => {
                            let message = format!(
                                "Error: Failed to frame command `{trimmed}`: {}",
                                format_transport_error(err)
                            );
                            let _ = writer_action_tx.send(Action::IncomingMessage(message));
                        }
                    },
                    Err(error) => {
                        let message = format!(
                            "Error: Failed to encode command `{trimmed}`: {}",
                            format_encode_error(error)
                        );
                        let _ = writer_action_tx.send(Action::IncomingMessage(message));
                    }
                }
            }
        });

        let mut reader = BufReader::new(reader_half);
        let mut pending = Vec::new();
        let mut read_buffer = [0u8; 512];
        'reader: loop {
            match reader.read(&mut read_buffer).await {
                Ok(0) => {
                    let _ = action_tx
                        .send(Action::ConnectionFailed("Serial connection closed.".into()));
                    break;
                }
                Ok(n) => {
                    pending.extend_from_slice(&read_buffer[..n]);
                    loop {
                        match try_decode_transport_frame(&pending) {
                            Ok(Some((payload, consumed))) => {
                                pending.drain(..consumed);
                                let message = payload_to_message(payload);
                                let _ = action_tx.send(Action::IncomingMessage(message));
                            }
                            Ok(None) => break,
                            Err(err) => {
                                let _ = action_tx.send(Action::ConnectionFailed(format!(
                                    "Failed to decode frame: {}",
                                    format_transport_error(err)
                                )));
                                break 'reader;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = action_tx
                        .send(Action::ConnectionFailed(format!("Serial read failed: {e}")));
                    break;
                }
            }
        }

        let _ = writer_task.await;
    }
}

fn format_encode_error(error: EncodeError) -> String {
    match error {
        EncodeError::Empty => "command is empty".into(),
        EncodeError::UnknownMethod => "unknown method".into(),
        EncodeError::UnknownOperation => "unknown operation".into(),
        EncodeError::UnsupportedOperation { method, operation } => format!(
            "unsupported operation {:?} for method {:?}",
            operation, method
        ),
        EncodeError::MissingOperation => "missing operation keyword".into(),
        EncodeError::MissingArgument { index } => {
            format!("missing argument at position {}", index + 1)
        }
        EncodeError::UnexpectedArgument { index } => {
            format!("unexpected argument starting at position {}", index + 1)
        }
        EncodeError::InvalidArgument { index } => {
            format!("invalid argument at position {}", index + 1)
        }
        EncodeError::OutputTooSmall => "output buffer is too small".into(),
    }
}

fn format_transport_error(error: TransportCodecError) -> String {
    match error {
        TransportCodecError::Encode(err) => format!("encode error: {err}"),
        TransportCodecError::Decode(err) => format!("decode error: {err}"),
    }
}

fn payload_to_message(payload: Vec<u8>) -> String {
    match String::from_utf8(payload) {
        Ok(text) => text,
        Err(err) => bytes_to_hex(err.into_bytes()),
    }
}

fn bytes_to_hex(bytes: Vec<u8>) -> String {
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
