use alloc::vec::Vec;

use super::{EncodeError, parse_u8};

pub fn encode_i2c_read(remainder: &str, output: &mut Vec<u8>) -> Result<usize, EncodeError> {
    const EXPECTED_ARGS: usize = 3;

    let mut args = remainder.split_ascii_whitespace();
    let addr_str = args
        .next()
        .ok_or(EncodeError::MissingArgument { index: 0 })?;
    let register_str = args
        .next()
        .ok_or(EncodeError::MissingArgument { index: 1 })?;
    let length_str = args
        .next()
        .ok_or(EncodeError::MissingArgument { index: 2 })?;

    if args.next().is_some() {
        return Err(EncodeError::UnexpectedArgument {
            index: EXPECTED_ARGS,
        });
    }

    let address = parse_u8(addr_str, 0)?;
    let register = parse_u8(register_str, 1)?;
    let length = parse_u8(length_str, 2)?;

    output.reserve(4);
    output.push(address);
    output.push(register);
    output.push(length);

    Ok(output.len())
}

pub fn encode_i2c_write(remainder: &str, output: &mut Vec<u8>) -> Result<usize, EncodeError> {
    let mut args = remainder.split_ascii_whitespace();
    let addr_str = args
        .next()
        .ok_or(EncodeError::MissingArgument { index: 0 })?;
    let register_str = args
        .next()
        .ok_or(EncodeError::MissingArgument { index: 1 })?;

    let payload_tokens: Vec<&str> = args.collect();
    if payload_tokens.is_empty() {
        return Err(EncodeError::MissingArgument { index: 2 });
    }
    if payload_tokens.len() > u8::MAX as usize {
        return Err(EncodeError::InvalidArgument { index: 2 });
    }

    let address = parse_u8(addr_str, 0)?;
    let register = parse_u8(register_str, 1)?;

    output.reserve(3 + payload_tokens.len());
    output.push(address);
    output.push(register);
    output.push(payload_tokens.len() as u8);

    for (i, token) in payload_tokens.into_iter().enumerate() {
        let byte = parse_u8(token, 2 + i)?;
        output.push(byte);
    }

    Ok(output.len())
}
