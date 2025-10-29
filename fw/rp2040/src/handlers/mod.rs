pub mod echo;
pub mod i2c;
pub mod spi;
pub mod uart;

use heapless::Vec;

use crate::state::{CommandOwned, Error};
use crate::MAX_COMMAND_SIZE;

pub fn execute_command(
    command: CommandOwned,
    response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>,
) -> Result<(), Error> {
    match command {
        CommandOwned::EchoWrite(payload) => echo::execute(payload.as_slice(), response_buf),
        CommandOwned::I2cRead {
            address,
            register,
            length,
        } => i2c::execute(address, register, length, response_buf),
    }
}
