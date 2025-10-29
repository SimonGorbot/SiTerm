#![no_std]
#![no_main]

use core::str;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};
use heapless::Vec;
use protocol::{
    transport::{self, FrameError, PostcardError},
    Command, HANDSHAKE_COMMAND, HANDSHAKE_DELIMITER, HANDSHAKE_RESPONSE, decode_command,
};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

const READ_BUFFER_SIZE: usize = 64;
const HANDSHAKE_BUFFER_SIZE: usize = 64;
const ECHO_PREFIX: &[u8] = b"rp2040: ";
const FRAME_BUFFER_SIZE: usize = 512;
const MAX_COMMAND_SIZE: usize = 256;
const ENCODED_FRAME_BUFFER_SIZE: usize = 320;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // USB CDC needs the USB peripheral and its interrupt handler.
    let driver = Driver::new(p.USB, Irqs);

    let mut config = Config::new(0x2e8a, 0x000a);
    config.manufacturer = Some("SiTerm");
    config.product = Some("RP2040 Zero CDC");
    config.serial_number = Some("0001");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Descriptor/state buffers must live for the lifetime of the USB device.
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut control_buf = [0; 64];
    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut [], // No Microsoft OS descriptors
        &mut control_buf,
    );

    // CDC-ACM class exposes a USB serial port to the host.
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);
    let mut device = builder.build();

    let usb_fut = device.run();

    let serial_fut = async {
        let delimiter = HANDSHAKE_DELIMITER.as_bytes();
        let mut read_buf = [0u8; READ_BUFFER_SIZE];

        loop {
            class.wait_connection().await;

            let mut handshake_buffer: Vec<u8, HANDSHAKE_BUFFER_SIZE> = Vec::new();
            let mut pending_frames: Vec<u8, FRAME_BUFFER_SIZE> = Vec::new();
            let mut handshake_complete = false;

            'connected: loop {
                let len = match class.read_packet(&mut read_buf).await {
                    Ok(len) => len,
                    Err(EndpointError::Disabled) => break 'connected,
                    Err(EndpointError::BufferOverflow) => continue,
                };

                if len == 0 {
                    continue;
                }

                if handshake_complete {
                    if let Err(err) =
                        process_transport_bytes(&mut class, &mut pending_frames, &read_buf[..len])
                            .await
                    {
                        if matches!(err, EndpointError::Disabled) {
                            break 'connected;
                        }
                    }
                    continue;
                }

                let mut idx = 0usize;
                while idx < len {
                    let byte = read_buf[idx];
                    idx += 1;

                    if handshake_buffer.push(byte).is_err() {
                        handshake_buffer.clear();
                    }

                    let buffer = handshake_buffer.as_slice();

                    let command_ready = if delimiter.is_empty() {
                        buffer.len() >= HANDSHAKE_COMMAND.len()
                    } else {
                        buffer.len() >= delimiter.len()
                            && &buffer[buffer.len() - delimiter.len()..] == delimiter
                    };

                    if command_ready {
                        let command_len = if delimiter.is_empty() {
                            buffer.len()
                        } else {
                            buffer.len() - delimiter.len()
                        };
                        let command_matches = str::from_utf8(&buffer[..command_len])
                            .map(|cmd| cmd == HANDSHAKE_COMMAND)
                            .unwrap_or(false);

                        handshake_buffer.clear();

                        if command_matches {
                            if let Err(err) =
                                write_packet_with_retry(&mut class, HANDSHAKE_RESPONSE.as_bytes())
                                    .await
                            {
                                if matches!(err, EndpointError::Disabled) {
                                    break 'connected;
                                }
                            } else {
                                handshake_complete = true;
                                pending_frames.clear();

                                if idx < len {
                                    if let Err(err) = process_transport_bytes(
                                        &mut class,
                                        &mut pending_frames,
                                        &read_buf[idx..len],
                                    )
                                    .await
                                    {
                                        if matches!(err, EndpointError::Disabled) {
                                            break 'connected;
                                        }
                                    }
                                }
                            }

                            break;
                        }
                    }
                }
            }
        }
    };

    join(usb_fut, serial_fut).await;
}

async fn write_packet_with_retry<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    data: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    loop {
        match class.write_packet(data).await {
            Ok(()) => return Ok(()),
            Err(EndpointError::BufferOverflow) => Timer::after_millis(10).await,
            Err(err) => return Err(err),
        }
    }
}

async fn process_transport_bytes<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    pending_frames: &mut Vec<u8, FRAME_BUFFER_SIZE>,
    data: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    if pending_frames.extend_from_slice(data).is_err() {
        pending_frames.clear();
        return Ok(());
    }

    loop {
        match transport::take_from_bytes(pending_frames.as_slice()) {
            Ok((frame, remaining)) => {
                let consumed = pending_frames.len() - remaining.len();
                let mut command_buf: Vec<u8, MAX_COMMAND_SIZE> = Vec::new();

                if command_buf.extend_from_slice(frame.payload).is_err() {
                    drop_prefix(pending_frames, consumed);
                    continue;
                }

                if let Ok(command) = decode_command(command_buf.as_slice()) {
                    handle_command(class, command).await?;
                }

                drop_prefix(pending_frames, consumed);
            }
            Err(FrameError::Deserialize(err)) => {
                if matches!(err, PostcardError::DeserializeUnexpectedEnd) {
                    break;
                } else {
                    pending_frames.clear();
                    break;
                }
            }
            Err(FrameError::Serialize(_)) => {
                pending_frames.clear();
                break;
            }
        }
    }

    Ok(())
}

async fn handle_command<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    command: Command<'_>,
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    match command {
        Command::EchoWrite { payload } => send_echo_response(class, payload).await?,
        _ => {}
    }

    Ok(())
}

async fn send_echo_response<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    payload: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    let mut response: Vec<u8, MAX_COMMAND_SIZE> = Vec::new();
    if response.extend_from_slice(ECHO_PREFIX).is_err() {
        return Ok(());
    }
    if response.extend_from_slice(payload).is_err() {
        return Ok(());
    }

    send_framed_payload(class, response.as_slice()).await
}

async fn send_framed_payload<'d, D>(
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

fn drop_prefix<const N: usize>(buffer: &mut Vec<u8, N>, count: usize) {
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
