// use crate::state::Error;
// use crate::MAX_COMMAND_SIZE;
// use defmt;
// use embassy_rp::i2c::{Async, Error as I2cError, I2c};
// use embassy_rp::peripherals::I2C1;
// use heapless::Vec;

// #[allow(unused_variables)]
// pub async fn execute_read(
//     address: u8,
//     register: u8,
//     length: u8,
//     response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>,
//     i2c: &mut I2c<'static, I2C1, Async>,
// ) -> Result<(), Error> {
//     // let mut result: [u8; 2] = [0, 0];
//     i2c.read_async(address, &mut result)
//         .await
//         .map_err(|_| Error::ExecutionFailed)

//     // fn map_i2c_err(e: I2cError) -> Error {
//     //     match e {
//     //         I2cError::Abort(abort_reason) => Error::ExecutionFailed,
//     //         I2cError::InvalidReadBufferLength => Error::InvalidChecksum,
//     //         I2cError::AddressOutOfRange(_) => Error::Timeout,
//     //         _ => Error::UnknownCommand,
//     //     }
//     // }

//     // i2c.read_async(address, response_buf)
//     //     .await
//     //     .map_err(|e| map_i2c_err(e))

//     // response_buf
//     //     .extend_from_slice(&buffer)
//     //     .map_err(|_| Error::Timeout)

//     // response_buf
//     //     .extend_from_slice(&[address, register, length])
//     //     .map_err(|_| Error::ExecutionFailed)
//     // match i2c.read_async(address, response_buf).await {
//     //     Ok(_) => response_buf
//     //         .extend_from_slice(b"g")
//     //         .map_err(|_| Error::ExecutionFailed),
//     //     // Err(I2cError::Abort(_)) => response_buf
//     //     //     .extend_from_slice(b"abort")
//     //     //     .map_err(|_| Error::ExecutionFailed),
//     //     // Err(I2cError::AddressOutOfRange(_)) => response_buf
//     //     //     .extend_from_slice(b"aor")
//     //     //     .map_err(|_| Error::ExecutionFailed),
//     //     // Err(I2cError::InvalidReadBufferLength) => response_buf
//     //     //     .extend_from_slice(b"irb")
//     //     //     .map_err(|_| Error::ExecutionFailed),
//     //     // Err(I2cError::InvalidWriteBufferLength) => response_buf
//     //     //     .extend_from_slice(b"iwb")
//     //     //     .map_err(|_| Error::ExecutionFailed),
//     //     Err(_) => response_buf
//     //         .extend_from_slice(b"ukn")
//     //         .map_err(|_| Error::ExecutionFailed),
//     // }
//     // .map_err(|_| Error::ExecutionFailed)
//     // Err(Error::ExecutionFailed)
// }

use crate::state::Error;
use crate::MAX_COMMAND_SIZE;
use embassy_rp::i2c::{Async, Error as I2cError, I2c};
use embassy_rp::peripherals::I2C1;
use heapless::Vec;

pub fn execute_read(
    address: u8,
    register: u8,
    length: u8,
    response: &mut Vec<u8, MAX_COMMAND_SIZE>,
    bus: &mut I2c<'static, I2C1, Async>,
) -> Result<(), Error> {
    fn map_i2c_err(e: I2cError) -> Error {
        match e {
            I2cError::Abort(abort_reason) => Error::ExecutionFailed,
            I2cError::InvalidReadBufferLength => Error::InvalidChecksum,
            I2cError::AddressOutOfRange(_) => Error::Timeout,
            _ => Error::UnknownCommand,
        }
    }

    let mut buf = [0u8];
    bus.blocking_write(address, &[register])
        .map_err(|e| map_i2c_err(e))?;
    bus.blocking_read(address, &mut buf)
        .map_err(|e| map_i2c_err(e))?;
    response
        .extend_from_slice(&buf)
        .map_err(|_| Error::InvalidChecksum)?;
    Ok(())
}
