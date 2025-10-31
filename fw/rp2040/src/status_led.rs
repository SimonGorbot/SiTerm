use embassy_futures::select::{select, Either};
use embassy_rp::pio::Instance;
use embassy_rp::pio_programs::ws2812::PioWs2812;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use smart_leds::RGB8;

pub const DEFAULT_NUM_LEDS: usize = 1;
pub const DEFAULT_BLINK_PERIOD: Duration = Duration::from_millis(600);
pub const ERROR_BLINK_PERIOD: Duration = Duration::from_millis(350);
pub const SUCCESS_BLINK_PERIOD: Duration = Duration::from_millis(100);
pub const HANDSHAKE_BLINK_PERIOD: Duration = Duration::from_millis(700);
pub const COMMUNICATION_PULSE_PERIOD: Duration = Duration::from_millis(800);
pub const ERROR_HOLD_DURATION: Duration = Duration::from_millis(800);
pub const SUCCESS_HOLD_DURATION: Duration = Duration::from_millis(400);
pub const WARNING_HOLD_DURATION: Duration = Duration::from_millis(500);

static STATUS_SIGNAL: Signal<CriticalSectionRawMutex, StatusPattern> = Signal::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusColours {
    Error,
    Warning,
    Communicating,
    Success,
    Idle,
}

impl StatusColours {
    pub const fn as_rgb(&self) -> RGB8 {
        match self {
            StatusColours::Error => RGB8::new(0, 150, 0),
            StatusColours::Warning => RGB8::new(80, 120, 0),
            StatusColours::Communicating => RGB8::new(0, 40, 80),
            StatusColours::Success => RGB8::new(120, 0, 0),
            StatusColours::Idle => RGB8::new(0, 0, 60),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusPattern {
    Solid(StatusColours),
    Blink {
        colour: StatusColours,
        period: Duration,
    },
    Pulse {
        colour: StatusColours,
        period: Duration,
    },
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
        Self { led: pio_ws2812 }
    }

    pub async fn set_colour(&mut self, colour: StatusColours) {
        self.set_rgb(colour.as_rgb()).await;
    }

    pub async fn set_rgb(&mut self, colour: RGB8) {
        self.led.write(&[colour; N]).await;
    }
}

pub fn signal(pattern: StatusPattern) {
    STATUS_SIGNAL.signal(pattern);
}

pub async fn drive<'d, P, const S: usize, const N: usize>(mut led: StatusLed<'d, P, S, N>) -> !
where
    P: Instance,
{
    let mut pattern = STATUS_SIGNAL.wait().await;

    'pattern: loop {
        match pattern {
            StatusPattern::Solid(colour) => {
                led.set_colour(colour).await;
                pattern = STATUS_SIGNAL.wait().await;
            }
            StatusPattern::Blink { colour, period } => {
                let on_rgb = colour.as_rgb();
                let off_rgb = RGB8::new(0, 0, 0);
                let half_period = nonzero_duration(period / 2);

                loop {
                    if let Some(new_pattern) = STATUS_SIGNAL.try_take() {
                        pattern = new_pattern;
                        continue 'pattern;
                    }

                    led.set_rgb(on_rgb).await;
                    if let Some(new_pattern) = wait_for_update(half_period).await {
                        pattern = new_pattern;
                        continue 'pattern;
                    }

                    if let Some(new_pattern) = STATUS_SIGNAL.try_take() {
                        pattern = new_pattern;
                        continue 'pattern;
                    }

                    led.set_rgb(off_rgb).await;
                    if let Some(new_pattern) = wait_for_update(half_period).await {
                        pattern = new_pattern;
                        continue 'pattern;
                    }
                }
            }
            StatusPattern::Pulse { colour, period } => {
                let base_rgb = colour.as_rgb();
                let up_len = PULSE_STEPS as u32;
                let down_len = up_len.saturating_sub(2);
                let cycle_steps = up_len + down_len;
                let step_duration = nonzero_duration(period / cycle_steps.max(1));
                let mut phase: u32 = 0;

                loop {
                    if let Some(new_pattern) = STATUS_SIGNAL.try_take() {
                        pattern = new_pattern;
                        continue 'pattern;
                    }

                    let idx = if phase < up_len {
                        phase as u8
                    } else {
                        let desc_phase = phase - up_len;
                        let descending_idx = up_len.saturating_sub(2 + desc_phase);
                        descending_idx as u8
                    };

                    let intensity = pulse_intensity(idx);
                    led.set_rgb(scale_rgb(base_rgb, intensity)).await;

                    if let Some(new_pattern) = wait_for_update(step_duration).await {
                        pattern = new_pattern;
                        continue 'pattern;
                    }

                    phase = if cycle_steps <= 1 {
                        0
                    } else {
                        (phase + 1) % cycle_steps
                    };
                }
            }
        }
    }
}

const PULSE_STEPS: u8 = 16;

fn pulse_intensity(step: u8) -> u8 {
    let max = PULSE_STEPS - 1;
    let clamped = step.min(max);
    ((clamped as u16 * 255) / max.max(1) as u16) as u8
}

fn scale_rgb(rgb: RGB8, scale: u8) -> RGB8 {
    RGB8::new(
        scale_channel(rgb.r, scale),
        scale_channel(rgb.g, scale),
        scale_channel(rgb.b, scale),
    )
}

fn scale_channel(channel: u8, scale: u8) -> u8 {
    ((channel as u16 * scale as u16) / 255) as u8
}

fn nonzero_duration(duration: Duration) -> Duration {
    if duration.as_ticks() == 0 {
        Duration::from_micros(1)
    } else {
        duration
    }
}

async fn wait_for_update(duration: Duration) -> Option<StatusPattern> {
    if duration.as_ticks() == 0 {
        return Some(STATUS_SIGNAL.wait().await);
    }

    match select(Timer::after(duration), STATUS_SIGNAL.wait()).await {
        Either::First(_) => None,
        Either::Second(pattern) => Some(pattern),
    }
}
