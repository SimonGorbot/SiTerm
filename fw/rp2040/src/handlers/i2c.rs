use crate::state::Error;
use crate::MAX_COMMAND_SIZE;
use core::fmt::Write;
use embassy_rp::i2c::{Async, Error as I2cError, I2c};
use embassy_rp::peripherals::I2C1;
use heapless::{String, Vec};

pub async fn execute_read(
    address: u8,
    register: u8,
    _length: u8,
    response: &mut Vec<u8, MAX_COMMAND_SIZE>,
    bus: &mut I2c<'static, I2C1, Async>,
) -> Result<(), Error> {
    // TODO: Implement batch read using length.

    fn push_i2c_error(
        response: &mut Vec<u8, MAX_COMMAND_SIZE>,
        err: I2cError,
    ) -> Result<(), Error> {
        response.clear();
        let mut tmp = String::<64>::new();
        write!(&mut tmp, "i2c error: {:?}", err).map_err(|_| Error::ExecutionFailed)?;
        response
            .extend_from_slice(tmp.as_bytes())
            .map_err(|_| Error::ExecutionFailed)
    }
    if let Err(err) = bus.write_async(address, [register]).await {
        let _ = push_i2c_error(response, err);
        return Err(Error::ExecutionFailed);
    }

    // TODO: Find out why write_async hangs. For now blocking works.
    // MRE: Replace bus.blocking_read(address, &mut buf) with bus.async_read(address, &mut buf).await
    let mut buf = [0u8];
    if let Err(err) = bus.blocking_read(address, &mut buf) {
        let _ = push_i2c_error(response, err);
        return Err(Error::ExecutionFailed);
    }

    response
        .extend_from_slice(&buf)
        .map_err(|_| Error::InvalidChecksum)?;
    Ok(())
}
