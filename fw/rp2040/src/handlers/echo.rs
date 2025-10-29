use heapless::Vec;

use crate::state::Error;
use crate::{ECHO_PREFIX, MAX_COMMAND_SIZE};

pub fn execute(payload: &[u8], response_buf: &mut Vec<u8, MAX_COMMAND_SIZE>) -> Result<(), Error> {
    response_buf
        .extend_from_slice(ECHO_PREFIX)
        .map_err(|_| Error::ExecutionFailed)?;
    response_buf
        .extend_from_slice(payload)
        .map_err(|_| Error::ExecutionFailed)?;
    Ok(())
}
