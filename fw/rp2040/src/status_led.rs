use core::default;

use embassy_rp::peripherals::{PIO0, USB};
use embassy_rp::pio::{Instance, InterruptHandler as PioInterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use embassy_rp::usb::{Driver, InterruptHandler as UsbInterruptHandler};
use smart_leds::{RGB, RGB8};

pub const DEFAULT_NUM_LEDS: usize = 1;

pub enum StatusColours {
    /// Red
    Error,
    /// Yellow
    Warning,
    /// Purple
    Communicating,
    /// Green
    Success,
    /// Blue
    Idle,
}

// I haven't figured out why but the red and green codes are switched.
impl StatusColours {
    pub const fn as_rbg(&self) -> RGB8 {
        match self {
            StatusColours::Error => RGB8::new(0, 100, 0),     // Red
            StatusColours::Warning => RGB8::new(100, 100, 0), // Yellow
            StatusColours::Communicating => RGB8::new(10, 70, 100), // Purple
            StatusColours::Success => RGB8::new(100, 0, 0),   // Green
            StatusColours::Idle => RGB8::new(0, 0, 100),      // Blue
        }
    }
}

pub struct StatusLed<'d, P, const S: usize, const N: usize>
where
    P: Instance,
{
    led: PioWs2812<'d, P, S, N>,
}

impl<'d, P, const S: usize, const N: usize> StatusLed<'d, P, S, N>
where
    P: Instance,
{
    pub fn new(pio_ws2812: PioWs2812<'d, P, S, N>) -> Self {
        StatusLed { led: pio_ws2812 }
    }

    pub async fn set_colour(&mut self, colour: StatusColours) {
        self.led.write(&[colour.as_rbg(); N]).await;
    }

    pub async fn turn_off(&mut self) {
        self.led.write(&[RGB8::new(0, 0, 0); N]).await;
    }
}
