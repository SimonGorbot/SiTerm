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
use protocol::{HANDSHAKE_COMMAND, HANDSHAKE_DELIMITER, HANDSHAKE_RESPONSE};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

const READ_BUFFER_SIZE: usize = 64;
const HANDSHAKE_BUFFER_SIZE: usize = 64;
const ECHO_PREFIX: &[u8] = b"rp2040: ";

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
                    if let Err(err) = echo_post_operation_bytes(&mut class, &read_buf[..len]).await
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

                                if idx < len {
                                    if let Err(err) =
                                        echo_post_operation_bytes(&mut class, &read_buf[idx..len])
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

async fn echo_post_operation_bytes<'d, D>(
    class: &mut CdcAcmClass<'d, D>,
    payload: &[u8],
) -> Result<(), EndpointError>
where
    D: embassy_usb::driver::Driver<'d>,
{
    let post_operation = payload.get(2..).unwrap_or(&[]);
    if post_operation.is_empty() {
        return Ok(());
    }

    write_packet_with_retry(class, ECHO_PREFIX).await?;
    write_packet_with_retry(class, post_operation).await
}
