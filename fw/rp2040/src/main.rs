#![no_std]
#![no_main]

mod handlers;
mod state;
mod status_led;
mod usb_transport;

// Embassy provides the async runtime and executor setup for the RP2040.
use embassy_executor::Spawner;
use embassy_futures::{
    join::join3,
    select::{select, Either},
};
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::{PIO0, USB};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::{Duration, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};
use state::StateMachine;
use status_led::{StatusColours, StatusLed, StatusPattern, DEFAULT_NUM_LEDS};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
});

// Shared buffer sizes and protocol limits used by the transport/state machine modules.
pub(crate) const READ_BUFFER_SIZE: usize = 64;
pub(crate) const HANDSHAKE_BUFFER_SIZE: usize = 64;
pub(crate) const ECHO_PREFIX: &[u8] = b"rp2040: ";
pub(crate) const FRAME_BUFFER_SIZE: usize = 512;
pub(crate) const MAX_COMMAND_SIZE: usize = 256;
pub(crate) const ENCODED_FRAME_BUFFER_SIZE: usize = 320;
pub(crate) const WRITE_RETRY_TIMEOUT_MS: u64 = 250;
pub(crate) const STATUS_LED_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let mut pio = Pio::new(p.PIO0, Irqs);
    let program = PioWs2812Program::new(&mut pio.common);
    let status_led = StatusLed::new(PioWs2812::<PIO0, 0, DEFAULT_NUM_LEDS>::new(
        &mut pio.common,
        pio.sm0,
        p.DMA_CH0,
        p.PIN_16,
        &program,
    ));
    status_led::signal(StatusPattern::Solid(StatusColours::Idle));

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

    // USB device task runs independently from the serial state machine task.
    let usb_fut = device.run();

    let serial_fut = async {
        let mut read_buf = [0u8; READ_BUFFER_SIZE];
        let mut machine = StateMachine::new();

        // Service connections forever; each iteration waits for a new host session.
        loop {
            class.wait_connection().await;
            machine.reset();

            // Kick the state machine once so it can emit any immediate errors (e.g. timeout).
            if let Err(err) = machine.consume(&mut class, &[]).await {
                if matches!(err, EndpointError::Disabled) {
                    continue;
                }
            }

            'connected: loop {
                machine.tick();

                let mut wait = STATUS_LED_POLL_INTERVAL;
                if let Some(timeout) = machine.handshake_timeout_remaining() {
                    wait = timeout.min(wait);
                }

                let wait = nonzero_duration(wait);

                // Drive handshake timeouts and LED latch expiry by racing USB reads against a timer tick.
                let len_result = match select(Timer::after(wait), class.read_packet(&mut read_buf))
                    .await
                {
                    Either::First(_) => {
                        if let Some(timeout) = machine.handshake_timeout_remaining() {
                            if timeout.as_ticks() == 0 {
                                if let Err(err) = machine.handle_handshake_timeout(&mut class).await
                                {
                                    if matches!(err, EndpointError::Disabled) {
                                        break 'connected;
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    Either::Second(result) => result,
                };

                let len = match len_result {
                    Ok(len) => len,
                    Err(EndpointError::Disabled) => break 'connected,
                    Err(EndpointError::BufferOverflow) => {
                        // Surface overflows to the host rather than silently dropping bytes.
                        if let Err(err) = machine.handle_buffer_overflow(&mut class).await {
                            if matches!(err, EndpointError::Disabled) {
                                break 'connected;
                            }
                        }
                        continue;
                    }
                };

                if len == 0 {
                    // Zero-length packets keep the link alive but carry no data.
                    continue;
                }

                // Feed new bytes into the state machine; bail out if the host disconnects.
                if let Err(err) = machine.consume(&mut class, &read_buf[..len]).await {
                    if matches!(err, EndpointError::Disabled) {
                        break 'connected;
                    }
                }
            }
        }
    };

    let led_fut = status_led::drive(status_led);

    // Execute the USB driver task, serial state machine, and LED driver together.
    let _ = join3(usb_fut, serial_fut, led_fut).await;
}

fn nonzero_duration(duration: Duration) -> Duration {
    if duration.as_ticks() == 0 {
        Duration::from_micros(1)
    } else {
        duration
    }
}
