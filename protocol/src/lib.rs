/*
Commands are built from the following:
Method: i2c, spi, pwm, uart, etc
Operation: read, write, continuous alternatives?

The rest is protocol specific:
i2c will have address, register,
spi will have cs pin, register,
pwm will have duty cycle,
uart will have byte mode and string mode
*/

#![cfg_attr(not(test), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::time::Duration;

pub mod transport {
    use postcard;
    use serde::{Deserialize, Serialize};

    pub use postcard::Error as PostcardError;

    /// Small wrapper around a payload that gets serialized with postcard to
    /// provide framing for arbitrary byte streams.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Frame<'a> {
        #[serde(borrow)]
        pub payload: &'a [u8],
    }

    impl<'a> Frame<'a> {
        pub const fn new(payload: &'a [u8]) -> Self {
            Self { payload }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FrameError {
        Serialize(PostcardError),
        Deserialize(PostcardError),
    }

    pub fn encode_into(payload: &[u8], buffer: &mut [u8]) -> Result<usize, FrameError> {
        let frame = Frame::new(payload);
        postcard::to_slice(&frame, buffer)
            .map(|written| written.len())
            .map_err(FrameError::Serialize)
    }

    pub fn take_from_bytes<'a>(bytes: &'a [u8]) -> Result<(Frame<'a>, &'a [u8]), FrameError> {
        postcard::take_from_bytes::<Frame<'a>>(bytes).map_err(FrameError::Deserialize)
    }
}

pub const HANDSHAKE_COMMAND: &str = "SiTerm?";
pub const HANDSHAKE_RESPONSE: &str = "SiTerm v1.0";
pub const HANDSHAKE_DELIMITER: &str = "\n";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Method {
    Echo = 0x01,
    I2c = 0x02,
    Spi = 0x03,
    Uart = 0x04,
    Pwm = 0x05,
}

impl TryFrom<&str> for Method {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.eq_ignore_ascii_case("echo") {
            Ok(Self::Echo)
        } else if value.eq_ignore_ascii_case("i2c") {
            Ok(Self::I2c)
        } else if value.eq_ignore_ascii_case("spi") {
            Ok(Self::Spi)
        } else if value.eq_ignore_ascii_case("uart") {
            Ok(Self::Uart)
        } else if value.eq_ignore_ascii_case("pwm") {
            Ok(Self::Pwm)
        } else {
            Err(())
        }
    }
}

impl Method {
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            x if x == Self::Echo as u8 => Some(Self::Echo),
            x if x == Self::I2c as u8 => Some(Self::I2c),
            x if x == Self::Spi as u8 => Some(Self::Spi),
            x if x == Self::Uart as u8 => Some(Self::Uart),
            x if x == Self::Pwm as u8 => Some(Self::Pwm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation {
    Read = 0x01,
    Write = 0x02,
}

impl TryFrom<&str> for Operation {
    type Error = ();
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.eq_ignore_ascii_case("r") || value.eq_ignore_ascii_case("read") {
            Ok(Self::Read)
        } else if value.eq_ignore_ascii_case("w") || value.eq_ignore_ascii_case("write") {
            Ok(Self::Write)
        } else {
            Err(())
        }
    }
}

impl Operation {
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            x if x == Self::Read as u8 => Some(Self::Read),
            x if x == Self::Write as u8 => Some(Self::Write),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct CommandDefinition {
    pub method: Method,
    pub operation: Operation,
}

pub const COMMAND_DICTIONARY: &[CommandDefinition] = &[
    CommandDefinition {
        method: Method::Echo,
        operation: Operation::Write,
    },
    CommandDefinition {
        method: Method::I2c,
        operation: Operation::Read,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolError {
    Empty,
    UnknownMethod(u8),
    UnknownOperation(u8),
    UnsupportedOperation {
        method: Method,
        operation: Operation,
    },
    MalformedPayload {
        method: Method,
        operation: Operation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command<'a> {
    EchoWrite {
        payload: &'a [u8],
    },
    I2cRead {
        address: u8,
        register: u8,
        length: u8,
    },
}

pub fn decode_command(buffer: &[u8]) -> Result<Command<'_>, ProtocolError> {
    let (&method_byte, rest) = buffer.split_first().ok_or(ProtocolError::Empty)?;
    let method = Method::from_byte(method_byte).ok_or(ProtocolError::UnknownMethod(method_byte))?;

    let (&operation_byte, payload) = rest.split_first().ok_or(ProtocolError::Empty)?;
    let operation = Operation::from_byte(operation_byte)
        .ok_or(ProtocolError::UnknownOperation(operation_byte))?;

    match (method, operation) {
        (Method::Echo, Operation::Write) => Ok(Command::EchoWrite { payload }),
        (Method::I2c, Operation::Read) => {
            if payload.len() < 3 {
                return Err(ProtocolError::MalformedPayload { method, operation });
            }

            let address = payload[0];
            let register = payload[1];
            let length = payload[2];

            Ok(Command::I2cRead {
                address,
                register,
                length,
            })
        }
        _ => Err(ProtocolError::UnsupportedOperation { method, operation }),
    }
}

#[cfg(feature = "alloc")]
pub mod host;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_echo() {
        let payload = [
            Method::Echo.as_byte(),
            Operation::Write.as_byte(),
            0xAA,
            0xBB,
        ];
        let command = decode_command(&payload).unwrap();

        match command {
            Command::EchoWrite {
                payload: echo_payload,
            } => assert_eq!(echo_payload, &[0xAA, 0xBB]),
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn decode_i2c_read() {
        let payload = [
            Method::I2c.as_byte(),
            Operation::Read.as_byte(),
            0x80,
            0x11,
            0x04,
        ];
        let command = decode_command(&payload).unwrap();

        match command {
            Command::I2cRead {
                address,
                register,
                length,
            } => {
                assert_eq!(address, 0x80);
                assert_eq!(register, 0x11);
                assert_eq!(length, 0x04);
            }
            _ => panic!("unexpected variant"),
        }
    }

    #[test]
    fn decode_unknown_method() {
        let payload = [0xFF];
        let err = decode_command(&payload).unwrap_err();
        assert!(matches!(err, ProtocolError::UnknownMethod(0xFF)));
    }
}
