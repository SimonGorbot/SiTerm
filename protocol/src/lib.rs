#![cfg_attr(not(test), no_std)]

use core::time::Duration;

pub const HANDSHAKE_COMMAND: &str = "SiTerm?";
pub const HANDSHAKE_RESPONSE: &str = "SiTerm v1.0";
pub const HANDSHAKE_DELIMITER: &str = "\n";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
