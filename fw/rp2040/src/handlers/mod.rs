pub mod echo;
pub mod i2c;
pub mod spi;
pub mod uart;

use embassy_rp::i2c::Async;
use embassy_rp::peripherals::I2C1;
use heapless::Vec;

use crate::state::{CommandOwned, Error};
use crate::MAX_COMMAND_SIZE;

pub struct HandlerPeripherals {
    pub i2c: embassy_rp::i2c::I2c<'static, I2C1, Async>,
    // uart: Uart,
    // spi: Spi,
}

pub async fn execute_command(
    command: CommandOwned,
    response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>,
    peripherals: &mut HandlerPeripherals,
) -> Result<(), Error> {
    match command {
        CommandOwned::EchoWrite(payload) => echo::execute(payload.as_slice(), response_buf),
        CommandOwned::I2cRead {
            address,
            register,
            length,
        } => {
            i2c::execute_read(
                address,
                register,
                length,
                response_buf,
                &mut peripherals.i2c,
            )
            .await
        }
    }
}
