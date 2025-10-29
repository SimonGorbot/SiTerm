use embassy_time::{Duration, Instant, Timer};
use embassy_usb::class::cdc_acm::CdcAcmClass;
use embassy_usb::driver::EndpointError;
use heapless::Vec;
use protocol::transport;

use crate::{ENCODED_FRAME_BUFFER_SIZE, READ_BUFFER_SIZE, WRITE_RETRY_TIMEOUT_MS};

pub async fn write_packet_with_retry<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    data: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    let deadline = Instant::now() + Duration::from_millis(WRITE_RETRY_TIMEOUT_MS);
    loop {
        match class.write_packet(data).await {
            Ok(()) => return Ok(()),
            Err(EndpointError::BufferOverflow) => {
                if Instant::now() >= deadline {
                    return Err(EndpointError::BufferOverflow);
                }
                Timer::after_millis(10).await;
            }
            Err(err) => return Err(err),
        }
    }
}

pub async fn send_framed_payload<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    payload: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    let mut frame_buf = [0u8; ENCODED_FRAME_BUFFER_SIZE];
    let len = match transport::encode_into(payload, &mut frame_buf) {
        Ok(len) => len,
        Err(_) => return Ok(()),
    };

    let mut offset = 0;
    while offset < len {
        let end = (offset + READ_BUFFER_SIZE).min(len);
        write_packet_with_retry(class, &frame_buf[offset..end]).await?;
        offset = end;
    }

    Ok(())
}

pub fn drop_prefix<const N: usize>(buffer: &mut Vec<u8, N>, count: usize) {
    if count == 0 {
        return;
    }
    if count >= buffer.len() {
        buffer.clear();
        return;
    }

    let len = buffer.len();
    for idx in count..len {
        buffer[idx - count] = buffer[idx];
    }
    buffer.truncate(len - count);
}
