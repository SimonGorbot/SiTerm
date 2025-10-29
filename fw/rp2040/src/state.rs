use core::str;

use embassy_time::{Duration, Instant};
use embassy_usb::class::cdc_acm::CdcAcmClass;
use embassy_usb::driver::EndpointError;
use heapless::Vec;
use protocol::{
    decode_command,
    transport::{self, FrameError, PostcardError},
    Command, HANDSHAKE_COMMAND, HANDSHAKE_DELIMITER, HANDSHAKE_RESPONSE, HANDSHAKE_TIMEOUT,
};

use crate::handlers;
use crate::usb_transport::{drop_prefix, send_framed_payload, write_packet_with_retry};
use crate::{FRAME_BUFFER_SIZE, HANDSHAKE_BUFFER_SIZE, MAX_COMMAND_SIZE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemState {
    Init,
    WaitForHandshake,
    WaitForMessage,
    ParseCommand,
    ExecuteAction,
    SendResponse,
    Error(Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidChecksum,
    UnknownCommand,
    Timeout,
    ExecutionFailed,
}

impl Error {
    const fn as_str(self) -> &'static str {
        match self {
            Error::InvalidChecksum => "InvalidChecksum",
            Error::UnknownCommand => "UnknownCommand",
            Error::Timeout => "Timeout",
            Error::ExecutionFailed => "ExecutionFailed",
        }
    }

    pub const fn as_bytes(self) -> &'static [u8] {
        self.as_str().as_bytes()
    }
}

pub enum CommandOwned {
    EchoWrite(Vec<u8, MAX_COMMAND_SIZE>),
    I2cRead {
        address: u8,
        register: u8,
        length: u8,
    },
}

impl CommandOwned {
    pub fn from_command(command: Command<'_>) -> Result<Self, Error> {
        match command {
            Command::EchoWrite { payload } => {
                let mut buffer: Vec<u8, MAX_COMMAND_SIZE> = Vec::new();
                buffer
                    .extend_from_slice(payload)
                    .map_err(|_| Error::ExecutionFailed)?;
                Ok(CommandOwned::EchoWrite(buffer))
            }
            Command::I2cRead {
                address,
                register,
                length,
            } => Ok(CommandOwned::I2cRead {
                address,
                register,
                length,
            }),
        }
    }
}

pub struct StateMachine {
    state: SystemState,
    handshake_buf: Vec<u8, HANDSHAKE_BUFFER_SIZE>,
    frame_buf: Vec<u8, FRAME_BUFFER_SIZE>,
    command_buf: Vec<u8, MAX_COMMAND_SIZE>,
    response_buf: Vec<u8, MAX_COMMAND_SIZE>,
    pending_command: Option<CommandOwned>,
    handshake_deadline: Option<Instant>,
    handshake_complete: bool,
}

impl StateMachine {
    pub const fn new() -> Self {
        Self {
            state: SystemState::Init,
            handshake_buf: Vec::new(),
            frame_buf: Vec::new(),
            command_buf: Vec::new(),
            response_buf: Vec::new(),
            pending_command: None,
            handshake_deadline: None,
            handshake_complete: false,
        }
    }

    pub fn reset(&mut self) {
        self.state = SystemState::Init;
        self.handshake_buf.clear();
        self.frame_buf.clear();
        self.command_buf.clear();
        self.response_buf.clear();
        self.pending_command = None;
        self.handshake_complete = false;
        self.schedule_handshake_deadline();
    }

    pub async fn consume<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
        data: &[u8],
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        self.advance(class).await?;

        for &byte in data {
            match self.state {
                SystemState::WaitForHandshake => self.step_handshake(class, byte).await?,
                SystemState::WaitForMessage => {
                    if self.frame_buf.push(byte).is_err() {
                        self.enter_error(Error::InvalidChecksum);
                    }
                }
                _ => {}
            }

            self.advance(class).await?;
        }

        self.advance(class).await
    }

    async fn step_handshake<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
        byte: u8,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        if self.handshake_buf.push(byte).is_err() {
            self.handshake_buf.clear();
            return Ok(());
        }

        let delimiter = HANDSHAKE_DELIMITER.as_bytes();
        let buffer = self.handshake_buf.as_slice();

        let command_ready = if delimiter.is_empty() {
            buffer.len() >= HANDSHAKE_COMMAND.len()
        } else {
            buffer.len() >= delimiter.len()
                && &buffer[buffer.len() - delimiter.len()..] == delimiter
        };

        if !command_ready {
            return Ok(());
        }

        let command_len = if delimiter.is_empty() {
            buffer.len()
        } else {
            buffer.len() - delimiter.len()
        };
        let command_matches = str::from_utf8(&buffer[..command_len])
            .map(|cmd| cmd == HANDSHAKE_COMMAND)
            .unwrap_or(false);

        self.handshake_buf.clear();

        if command_matches {
            write_packet_with_retry(class, HANDSHAKE_RESPONSE.as_bytes()).await?;
            self.frame_buf.clear();
            self.handshake_complete = true;
            self.handshake_deadline = None;
            self.state = SystemState::WaitForMessage;
        }

        Ok(())
    }

    async fn advance<'d, D>(&mut self, class: &mut CdcAcmClass<'d, D>) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        loop {
            match self.state {
                SystemState::Init => {
                    if self.handshake_deadline.is_none() {
                        self.schedule_handshake_deadline();
                    }
                    self.state = SystemState::WaitForHandshake;
                }
                SystemState::WaitForHandshake => return Ok(()),
                SystemState::WaitForMessage => match self.take_ready_frame() {
                    Ok(Some(())) => {
                        self.state = SystemState::ParseCommand;
                    }
                    Ok(None) => return Ok(()),
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::ParseCommand => match self.decode_pending_command() {
                    Ok(()) => {
                        self.state = SystemState::ExecuteAction;
                    }
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::ExecuteAction => match self.perform_command() {
                    Ok(()) => {
                        self.state = SystemState::SendResponse;
                    }
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::SendResponse => {
                    self.flush_response(class).await?;
                    self.state = SystemState::WaitForMessage;
                }
                SystemState::Error(err) => {
                    self.flush_error(class, err).await?;
                    if self.handshake_complete {
                        self.state = SystemState::WaitForMessage;
                    } else {
                        self.schedule_handshake_deadline();
                        self.state = SystemState::WaitForHandshake;
                    }
                }
            }
        }
    }

    fn take_ready_frame(&mut self) -> Result<Option<()>, Error> {
        match transport::take_from_bytes(self.frame_buf.as_slice()) {
            Ok((frame, remaining)) => {
                let consumed = self.frame_buf.len() - remaining.len();
                self.command_buf.clear();
                if self.command_buf.extend_from_slice(frame.payload).is_err() {
                    self.frame_buf.clear();
                    return Err(Error::InvalidChecksum);
                }

                drop_prefix(&mut self.frame_buf, consumed);
                Ok(Some(()))
            }
            Err(FrameError::Deserialize(err)) => {
                if matches!(err, PostcardError::DeserializeUnexpectedEnd) {
                    Ok(None)
                } else {
                    self.frame_buf.clear();
                    Err(Error::InvalidChecksum)
                }
            }
            Err(FrameError::Serialize(_)) => {
                self.frame_buf.clear();
                Err(Error::InvalidChecksum)
            }
        }
    }

    fn decode_pending_command(&mut self) -> Result<(), Error> {
        match decode_command(self.command_buf.as_slice()) {
            Ok(command) => {
                let owned = CommandOwned::from_command(command)?;
                self.pending_command = Some(owned);
                self.command_buf.clear();
                Ok(())
            }
            Err(err) => Err(Self::map_protocol_error(err)),
        }
    }

    fn perform_command(&mut self) -> Result<(), Error> {
        if let Some(command) = self.pending_command.take() {
            self.response_buf.clear();
            handlers::execute_command(command, &mut self.response_buf)
        } else {
            Ok(())
        }
    }

    async fn flush_response<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        send_framed_payload(class, self.response_buf.as_slice()).await?;
        self.response_buf.clear();
        Ok(())
    }

    async fn flush_error<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
        err: Error,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        self.response_buf.clear();
        let _ = self.response_buf.extend_from_slice(b"ERR: ");
        let _ = self.response_buf.extend_from_slice(err.as_bytes());
        send_framed_payload(class, self.response_buf.as_slice()).await?;
        self.response_buf.clear();
        Ok(())
    }

    fn enter_error(&mut self, err: Error) {
        self.pending_command = None;
        self.command_buf.clear();
        self.state = SystemState::Error(err);
    }

    fn map_protocol_error(err: protocol::ProtocolError) -> Error {
        match err {
            protocol::ProtocolError::Empty => Error::InvalidChecksum,
            protocol::ProtocolError::MalformedPayload { .. } => Error::InvalidChecksum,
            protocol::ProtocolError::UnknownMethod(_) => Error::UnknownCommand,
            protocol::ProtocolError::UnknownOperation(_) => Error::UnknownCommand,
            protocol::ProtocolError::UnsupportedOperation { .. } => Error::UnknownCommand,
        }
    }

    fn handshake_timeout_duration() -> Duration {
        let millis = HANDSHAKE_TIMEOUT.as_millis() as u64;
        Duration::from_millis(millis)
    }

    fn schedule_handshake_deadline(&mut self) {
        self.handshake_deadline = Some(Instant::now() + Self::handshake_timeout_duration());
    }

    pub fn handshake_timeout_remaining(&self) -> Option<Duration> {
        if self.handshake_complete {
            return None;
        }
        if !matches!(
            self.state,
            SystemState::WaitForHandshake | SystemState::Init
        ) {
            return None;
        }
        let deadline = self.handshake_deadline?;
        let now = Instant::now();
        if deadline <= now {
            Some(Duration::from_micros(0))
        } else {
            Some(deadline - now)
        }
    }

    pub async fn handle_handshake_timeout<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        self.handshake_buf.clear();
        self.frame_buf.clear();
        self.handshake_complete = false;
        self.schedule_handshake_deadline();
        self.enter_error(Error::Timeout);
        self.advance(class).await
    }

    pub async fn handle_buffer_overflow<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        self.frame_buf.clear();
        self.enter_error(Error::InvalidChecksum);
        self.advance(class).await
    }
}
