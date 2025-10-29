use alloc::vec::Vec;
use postcard::{self, Error as PostcardError};

use crate::{
    transport::{self, Frame as TransportFrame, FrameError},
    COMMAND_DICTIONARY, Method, Operation,
};

pub mod i2c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    Empty,
    UnknownMethod,
    UnknownOperation,
    UnsupportedOperation {
        method: Method,
        operation: Operation,
    },
    MissingOperation,
    MissingArgument {
        index: usize,
    },
    UnexpectedArgument {
        index: usize,
    },
    InvalidArgument {
        index: usize,
    },
    OutputTooSmall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportCodecError {
    Encode(PostcardError),
    Decode(PostcardError),
}

pub fn encode_transport_frame(payload: &[u8]) -> Result<Vec<u8>, TransportCodecError> {
    let frame = TransportFrame::new(payload);
    postcard::to_allocvec(&frame).map_err(TransportCodecError::Encode)
}

pub fn try_decode_transport_frame(
    buffer: &[u8],
) -> Result<Option<(Vec<u8>, usize)>, TransportCodecError> {
    match transport::take_from_bytes(buffer) {
        Ok((frame, remaining)) => {
            let consumed = buffer.len() - remaining.len();
            Ok(Some((frame.payload.to_vec(), consumed)))
        }
        Err(FrameError::Deserialize(err)) => {
            if err == PostcardError::DeserializeUnexpectedEnd {
                Ok(None)
            } else {
                Err(TransportCodecError::Decode(err))
            }
        }
        Err(FrameError::Serialize(err)) => Err(TransportCodecError::Decode(err)),
    }
}

pub fn encode_command(input: &str) -> Result<Vec<u8>, EncodeError> {
    let mut buffer = Vec::with_capacity(input.len() + 1);
    let len = encode_command_into(input, &mut buffer)?;
    buffer.truncate(len);
    Ok(buffer)
}

pub fn encode_command_into(input: &str, output: &mut Vec<u8>) -> Result<usize, EncodeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(EncodeError::Empty);
    }

    let mut parts = trimmed.splitn(2, " ");
    let method_keyword = parts.next().unwrap_or("");
    let post_method_remaining = parts.next().unwrap_or("").trim_start();

    let method = Method::try_from(method_keyword).map_err(|_| EncodeError::UnknownMethod)?;

    let (operation, post_operation_remaining) = if method == Method::Echo {
        (Operation::Write, post_method_remaining)
    } else {
        if post_method_remaining.is_empty() {
            return Err(EncodeError::MissingOperation);
        }

        let mut op_parts = post_method_remaining.splitn(2, " ");
        let operation_keyword = op_parts.next().unwrap_or("");
        let remainder = op_parts.next().unwrap_or("").trim_start();

        let operation =
            Operation::try_from(operation_keyword).map_err(|_| EncodeError::UnknownOperation)?;
        (operation, remainder)
    };

    let supported = COMMAND_DICTIONARY
        .iter()
        .any(|def| def.method == method && def.operation == operation)
        || matches!(
            (method, operation),
            (Method::I2c, Operation::Read | Operation::Write)
        );

    if !supported {
        return Err(EncodeError::UnsupportedOperation { method, operation });
    }

    output.clear();

    output.push(method.as_byte());
    output.push(operation.as_byte());

    match (method, operation) {
        (Method::Echo, Operation::Write) => encode_echo(post_operation_remaining, output),
        (Method::I2c, Operation::Read) => i2c::encode_i2c_read(post_operation_remaining, output),
        (Method::I2c, Operation::Write) => i2c::encode_i2c_write(post_operation_remaining, output),
        _ => Err(EncodeError::UnsupportedOperation { method, operation }),
    }
}

fn encode_echo(remainder: &str, output: &mut Vec<u8>) -> Result<usize, EncodeError> {
    output.extend_from_slice(remainder.as_bytes());
    Ok(output.len())
}

pub(super) fn parse_u8(token: &str, index: usize) -> Result<u8, EncodeError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(EncodeError::MissingArgument { index });
    }

    let (radix, digits) = if let Some(stripped) = token.strip_prefix("0x") {
        (16, stripped)
    } else if let Some(stripped) = token.strip_prefix("0b") {
        (2, stripped)
    } else {
        (10, token)
    };

    if digits.is_empty() {
        return Err(EncodeError::InvalidArgument { index });
    }

    u8::from_str_radix(digits, radix).map_err(|_| EncodeError::InvalidArgument { index })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_echo_roundtrip() {
        let input = "echo hello world";
        let buf = encode_command(input).unwrap();
        assert_eq!(buf[0], Method::Echo.as_byte());
        assert_eq!(buf[1], Operation::Write.as_byte());
        assert_eq!(&buf[2..], b"hello world");
    }

    #[test]
    fn encode_i2c_read_hex_args() {
        let buf = encode_command("i2c read 0x80 0x11").unwrap();
        assert_eq!(
            buf,
            vec![Method::I2c.as_byte(), Operation::Read.as_byte(), 0x80, 0x11,]
        );
    }

    #[test]
    fn encode_i2c_read_errors_on_missing_argument() {
        let mut buf = Vec::new();
        let err = encode_command_into("i2c read 0x80", &mut buf).unwrap_err();
        assert!(matches!(err, EncodeError::MissingArgument { index: 1 }));
    }

    #[test]
    fn encode_i2c_write_basic() {
        let buf = encode_command("i2c write 0x80 0x11 0x01 0x02").unwrap();
        assert_eq!(
            buf,
            vec![
                Method::I2c.as_byte(),
                Operation::Write.as_byte(),
                0x80,
                0x11,
                0x02,
                0x01,
                0x02
            ]
        );
    }

    #[test]
    fn encode_unknown_command() {
        let err = encode_command("foo").unwrap_err();
        assert!(matches!(err, EncodeError::UnknownMethod));
    }

    #[test]
    fn transport_roundtrip() {
        let payload = vec![0xAA, 0x00, 0x55];
        let encoded = encode_transport_frame(&payload).unwrap();
        let (decoded, used) = try_decode_transport_frame(&encoded).unwrap().unwrap();
        assert_eq!(used, encoded.len());
        assert_eq!(decoded, payload);
    }
}
