use crate::state::Error;
use crate::MAX_COMMAND_SIZE;
use core::fmt::Write;
use embassy_rp::i2c::{Async, Error as I2cError, I2c};
use embassy_rp::peripherals::I2C1;
use heapless::{String, Vec};

fn push_error_message(
    response: &mut Vec<u8, MAX_COMMAND_SIZE>,
    message: &str,
) -> Result<(), Error> {
    response.clear();
    response
        .extend_from_slice(message.as_bytes())
        .map_err(|_| Error::BufferProcessFailed)
}

fn push_i2c_error(response: &mut Vec<u8, MAX_COMMAND_SIZE>, err: I2cError) -> Result<(), Error> {
    let mut tmp = String::<64>::new();
    write!(&mut tmp, "i2c error: {:?}", err).map_err(|_| Error::BufferProcessFailed)?;
    push_error_message(response, tmp.as_str())
}

pub async fn execute_read(
    address: u8,
    register: u8,
    length: u8,
    response: &mut Vec<u8, MAX_COMMAND_SIZE>,
    bus: &mut I2c<'static, I2C1, Async>,
) -> Result<(), Error> {
    let len = length as usize;
    let available_capacity = response.capacity().saturating_sub(response.len());
    if len == 0 {
        let _ = push_error_message(response, "i2c error: length must be greater than zero");
        return Err(Error::ExecutionFailed);
    }
    if len > available_capacity {
        let _ = push_error_message(response, "i2c error: length exceeds buffer");
        return Err(Error::ExecutionFailed);
    }

    let mut buf = [0u8; MAX_COMMAND_SIZE];
    let read_buf = &mut buf[..len];

    // Use a single transaction to write the register address then read the requested bytes.
    if let Err(err) = bus.blocking_write_read(address, &[register], read_buf) {
        let _ = push_i2c_error(response, err);
        return Err(Error::ExecutionFailed);
    }

    response
        .extend_from_slice(read_buf)
        .map_err(|_| Error::BufferProcessFailed)?;
    Ok(())
}

pub async fn execute_write(
    address: u8,
    register: u8,
    payload: &[u8],
    response: &mut Vec<u8, MAX_COMMAND_SIZE>,
    bus: &mut I2c<'static, I2C1, Async>,
) -> Result<(), Error> {
    if payload.is_empty() {
        let _ = push_error_message(response, "i2c error: payload must not be empty");
        return Err(Error::ExecutionFailed);
    }

    let total_len = payload.len() + 1; // include register byte
    if total_len > MAX_COMMAND_SIZE {
        let _ = push_error_message(response, "i2c error: payload too large");
        return Err(Error::ExecutionFailed);
    }

    let mut buf = [0u8; MAX_COMMAND_SIZE];
    buf[0] = register;
    buf[1..total_len].copy_from_slice(payload);

    if let Err(err) = bus.blocking_write(address, &buf[..total_len]) {
        let _ = push_i2c_error(response, err);
        return Err(Error::ExecutionFailed);
    }

    Ok(())
}
