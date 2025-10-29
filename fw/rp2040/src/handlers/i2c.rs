use heapless::Vec;

use crate::state::Error;
use crate::MAX_COMMAND_SIZE;

#[allow(unused_variables)]
pub fn execute(
    address: u8,
    register: u8,
    length: u8,
    _response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>,
) -> Result<(), Error> {
    Err(Error::ExecutionFailed)
}
