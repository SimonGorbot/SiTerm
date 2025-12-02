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

use crate::handlers::{self, HandlerPeripherals};
use crate::status_led::{
    self, StatusColours, StatusPattern, COMMUNICATION_PULSE_PERIOD, DEFAULT_BLINK_PERIOD,
    ERROR_BLINK_PERIOD, ERROR_HOLD_DURATION, HANDSHAKE_BLINK_PERIOD, SUCCESS_BLINK_PERIOD,
    SUCCESS_HOLD_DURATION, WARNING_HOLD_DURATION,
};
use crate::usb_transport::{drop_prefix, send_framed_payload, write_packet_with_retry};
use crate::{FRAME_BUFFER_SIZE, HANDSHAKE_BUFFER_SIZE, MAX_COMMAND_SIZE};

/// High-level states cycled through while talking to the tui host.
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

/// Errors surfaced to the host when parsing or executing a command fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidChecksum,
    UnknownCommand,
    Timeout,
    ExecutionFailed,
    BufferProcessFailed,
}

impl Error {
    const fn as_str(self) -> &'static str {
        match self {
            Error::InvalidChecksum => "InvalidChecksum",
            Error::UnknownCommand => "UnknownCommand",
            Error::Timeout => "Timeout",
            Error::ExecutionFailed => "ExecutionFailed",
            Error::BufferProcessFailed => "BufferProcessFailed",
        }
    }

    pub const fn as_bytes(self) -> &'static [u8] {
        self.as_str().as_bytes()
    }
}

/// Owned variants of protocol commands so handlers can borrow payloads without lifetime issues.
pub enum CommandOwned {
    EchoWrite(Vec<u8, MAX_COMMAND_SIZE>),
    I2cRead {
        address: u8,
        register: u8,
        length: u8,
    },
    I2cWrite {
        address: u8,
        register: u8,
        payload: Vec<u8, MAX_COMMAND_SIZE>,
    },
}

impl CommandOwned {
    /// Converts borrowed commands into owned versions for convenience.
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
            Command::I2cWrite {
                address,
                register,
                payload,
            } => {
                let mut buffer: Vec<u8, MAX_COMMAND_SIZE> = Vec::new();
                buffer
                    .extend_from_slice(payload)
                    .map_err(|_| Error::ExecutionFailed)?;

                Ok(CommandOwned::I2cWrite {
                    address,
                    register,
                    payload: buffer,
                })
            }
        }
    }
}

/// Tracks buffers, timers, and state transitions for the USB CDC control loop.
pub struct StateMachine {
    state: SystemState,
    handshake_buf: Vec<u8, HANDSHAKE_BUFFER_SIZE>,
    frame_buf: Vec<u8, FRAME_BUFFER_SIZE>,
    command_buf: Vec<u8, MAX_COMMAND_SIZE>,
    response_buf: Vec<u8, MAX_COMMAND_SIZE>,
    pending_command: Option<CommandOwned>,
    handshake_deadline: Option<Instant>,
    handshake_complete: bool,
    last_status_pattern: Option<StatusPattern>,
    latched_pattern: Option<LatchedPattern>,
    handler_peripherals: HandlerPeripherals,
}

#[derive(Clone, Copy)]
struct LatchedPattern {
    pattern: StatusPattern,
    until: Instant,
}

impl StateMachine {
    /// Create a state machine with empty buffers and no pending handshake.
    pub const fn new(handler_peripherals: HandlerPeripherals) -> Self {
        Self {
            state: SystemState::Init,
            handshake_buf: Vec::new(),
            frame_buf: Vec::new(),
            command_buf: Vec::new(),
            response_buf: Vec::new(),
            pending_command: None,
            handshake_deadline: None,
            handshake_complete: false,
            last_status_pattern: None,
            latched_pattern: None,
            handler_peripherals,
        }
    }

    /// Return to the initial states, clearing buffers and resetting deadlines.
    pub fn reset(&mut self) {
        self.handshake_buf.clear();
        self.frame_buf.clear();
        self.command_buf.clear();
        self.response_buf.clear();
        self.pending_command = None;
        self.handshake_complete = false;
        self.last_status_pattern = None;
        self.latched_pattern = None;
        self.handshake_deadline = None;
        self.schedule_handshake_deadline();
        self.set_state(SystemState::Init);
    }

    fn set_state(&mut self, state: SystemState) {
        self.state = state;
        self.refresh_status_led();
    }

    pub fn tick(&mut self) {
        self.refresh_status_led();
    }

    fn refresh_status_led(&mut self) {
        let now = Instant::now();

        if let Some(latch) = self.latched_pattern {
            if now >= latch.until {
                self.latched_pattern = None;
            }
        }

        let (pattern, hold) = self.state_pattern();

        if let Some(duration) = hold {
            self.latched_pattern = Some(LatchedPattern {
                pattern,
                until: now + duration,
            });
        }

        let effective = if let Some(latch) = self.latched_pattern {
            if now < latch.until {
                latch.pattern
            } else {
                self.latched_pattern = None;
                pattern
            }
        } else {
            pattern
        };

        if self.last_status_pattern != Some(effective) {
            status_led::signal(effective);
            self.last_status_pattern = Some(effective);
        }
    }

    fn state_pattern(&self) -> (StatusPattern, Option<Duration>) {
        match self.state {
            SystemState::Init => (StatusPattern::Solid(StatusColours::Idle), None),
            SystemState::WaitForHandshake => (
                StatusPattern::Blink {
                    colour: StatusColours::Warning,
                    period: HANDSHAKE_BLINK_PERIOD,
                },
                None,
            ),
            SystemState::WaitForMessage => (StatusPattern::Solid(StatusColours::Idle), None),
            SystemState::ParseCommand | SystemState::ExecuteAction => (
                StatusPattern::Pulse {
                    colour: StatusColours::Communicating,
                    period: COMMUNICATION_PULSE_PERIOD,
                },
                None,
            ),
            SystemState::SendResponse => (
                StatusPattern::Blink {
                    colour: StatusColours::Success,
                    period: SUCCESS_BLINK_PERIOD,
                },
                Some(SUCCESS_HOLD_DURATION),
            ),
            SystemState::Error(err) => match err {
                Error::Timeout => (
                    StatusPattern::Blink {
                        colour: StatusColours::Warning,
                        period: DEFAULT_BLINK_PERIOD,
                    },
                    Some(WARNING_HOLD_DURATION),
                ),
                _ => (
                    StatusPattern::Blink {
                        colour: StatusColours::Error,
                        period: ERROR_BLINK_PERIOD,
                    },
                    Some(ERROR_HOLD_DURATION),
                ),
            },
        }
    }

    /// Feed newly received bytes via USB into the FSM, progressing through handshake, parsing, and reply.
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
                        self.frame_buf.clear();
                        self.enter_error(Error::InvalidChecksum);
                    }
                }
                _ => {}
            }

            self.advance(class).await?;
        }

        self.advance(class).await
    }

    /// Consume a single handshake byte, answering with the handshake response once the delimiter matches.
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

        // Collects bytes until delimiter arrives.
        if buffer.len() < delimiter.len() || &buffer[buffer.len() - delimiter.len()..] != delimiter
        {
            return Ok(());
        }

        let command_len = buffer.len() - delimiter.len();
        let command_matches = str::from_utf8(&buffer[..command_len])
            .map(|cmd| cmd == HANDSHAKE_COMMAND)
            .unwrap_or(false);

        self.handshake_buf.clear();

        if command_matches {
            write_packet_with_retry(class, HANDSHAKE_RESPONSE.as_bytes()).await?;
            self.frame_buf.clear();
            self.handshake_complete = true;
            self.handshake_deadline = None;
            self.set_state(SystemState::WaitForMessage);
        }

        Ok(())
    }

    /// Drive the FSM forward until it needs more input or I/O completes, performing work for each state.
    async fn advance<'d, D>(&mut self, class: &mut CdcAcmClass<'d, D>) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        loop {
            self.refresh_status_led();
            match self.state {
                SystemState::Init => {
                    if self.handshake_deadline.is_none() {
                        self.schedule_handshake_deadline();
                    }
                    self.set_state(SystemState::WaitForHandshake);
                }
                SystemState::WaitForHandshake => return Ok(()),
                SystemState::WaitForMessage => match self.take_ready_frame() {
                    Ok(Some(())) => {
                        self.set_state(SystemState::ParseCommand);
                    }
                    Ok(None) => return Ok(()),
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::ParseCommand => match self.decode_pending_command() {
                    Ok(()) => {
                        self.set_state(SystemState::ExecuteAction);
                    }
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::ExecuteAction => match self.perform_command().await {
                    Ok(()) => {
                        self.set_state(SystemState::SendResponse);
                    }
                    Err(err) => {
                        self.enter_error(err);
                    }
                },
                SystemState::SendResponse => {
                    self.flush_response(class).await?;
                    self.set_state(SystemState::WaitForMessage);
                }
                SystemState::Error(err) => {
                    self.flush_error(class, err).await?;
                    if self.handshake_complete {
                        self.set_state(SystemState::WaitForMessage);
                    } else {
                        self.schedule_handshake_deadline();
                        self.set_state(SystemState::WaitForHandshake);
                    }
                }
            }
        }
    }

    /// Try to take one complete transport frame out of `frame_buf`.
    /// Returns `Ok(Some(()))` when a frame was removed and its payload copied into `command_buf`,
    /// `Ok(None)` when more bytes are required, and `Err(Error::InvalidChecksum)` when the buffered
    /// data is malformed (the frame buffer is cleared).
    fn take_ready_frame(&mut self) -> Result<Option<()>, Error> {
        match transport::take_from_bytes(self.frame_buf.as_slice()) {
            Ok((frame, remaining)) => {
                let consumed = self.frame_buf.len() - remaining.len(); // Bytes that belong to this frame.
                self.command_buf.clear();
                if self.command_buf.extend_from_slice(frame.payload).is_err() {
                    self.frame_buf.clear();
                    return Err(Error::InvalidChecksum); // Payload is too large for the command buffer therefore surface error.
                }

                drop_prefix(&mut self.frame_buf, consumed); // Leave any trailing bytes for the next frame.
                Ok(Some(()))
            }
            Err(FrameError::Deserialize(err)) => {
                if matches!(err, PostcardError::DeserializeUnexpectedEnd) {
                    Ok(None) // Frame is incomplete, wait for more bytes to arrive.
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

    /// Deserialize the buffered frame payload into a pending command the executor can own.
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

    /// Execute the pending command via the handler table and capture any response bytes.
    async fn perform_command(&mut self) -> Result<(), Error> {
        if let Some(command) = self.pending_command.take() {
            self.response_buf.clear();
            handlers::execute_command(
                command,
                &mut self.response_buf,
                &mut self.handler_peripherals,
            )
            .await
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

    /// Frame and transmit the buffered response payload to the tui host.
    async fn flush_error<'d, D>(
        &mut self,
        class: &mut CdcAcmClass<'d, D>,
        err: Error,
    ) -> Result<(), EndpointError>
    where
        D: embassy_usb::driver::Driver<'d>,
    {
        let mut errored_buffer = Vec::<u8, MAX_COMMAND_SIZE>::new();
        let _ = errored_buffer.extend_from_slice(self.response_buf.as_slice());
        self.response_buf.clear();

        let _ = self.response_buf.extend_from_slice(b"ERR: ");
        let _ = self.response_buf.extend_from_slice(err.as_bytes());
        if !errored_buffer.is_empty() {
            let _ = self.response_buf.extend_from_slice(b": ");
            let _ = self
                .response_buf
                .extend_from_slice(errored_buffer.as_slice());
        }

        send_framed_payload(class, self.response_buf.as_slice()).await?;
        self.response_buf.clear();
        Ok(())
    }

    /// Emit a framed `ERR: <name>` payload describing the provided error.
    fn enter_error(&mut self, err: Error) {
        self.pending_command = None;
        self.command_buf.clear();
        self.set_state(SystemState::Error(err));
    }

    // TODO: More comprehensive error surfacing.
    /// Map protocol-layer decoding failures onto user-visible error categories.
    fn map_protocol_error(err: protocol::ProtocolError) -> Error {
        match err {
            protocol::ProtocolError::Empty => Error::InvalidChecksum,
            protocol::ProtocolError::MalformedPayload { .. } => Error::InvalidChecksum,
            protocol::ProtocolError::UnknownMethod(_) => Error::UnknownCommand,
            protocol::ProtocolError::UnknownOperation(_) => Error::UnknownCommand,
            protocol::ProtocolError::UnsupportedOperation { .. } => Error::UnknownCommand,
        }
    }

    /// Sets the deadline for the handshake with tui host.
    fn schedule_handshake_deadline(&mut self) {
        let secs = HANDSHAKE_TIMEOUT.as_secs() as u64; // Need to convert from core::time::Duration to embassy_time::duration :/
        let hs_timeout = Duration::from_secs(secs);
        self.handshake_deadline = Some(Instant::now() + hs_timeout);
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

    /// Recover from a handshake timeout by clearing buffers and surfacing a timeout error frame.
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

    /// Recover from a USB buffer overflow by dropping partial frames and flagging an invalid checksum.
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
