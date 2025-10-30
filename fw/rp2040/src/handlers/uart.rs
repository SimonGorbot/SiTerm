use heapless::Vec;

use crate::state::Error;
use crate::MAX_COMMAND_SIZE;

#[allow(unused_variables, dead_code)]
pub fn execute(_response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>) -> Result<(), Error> {
    Err(Error::ExecutionFailed)
}
