use core::fmt;

use alloc::boxed::Box;
use embassy_executor::task;
use embassy_rp::bind_interrupts;
use alloc::vec::{Vec};
use alloc::vec;

use embassy_time::{Duration, Instant, Ticker};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{Instance, InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use smart_leds::RGB8;

use crate::config_resources::RGBLEDResources;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

const NUM_LEDS: usize = 5;

/// Input a value 0 to 255 to get a color value
/// The colours are a transition r - g - b - back to r.
fn wheel(mut wheel_pos: u8) -> RGB8 {
    wheel_pos = 255 - wheel_pos;
    if wheel_pos < 85 {
        return (255 - wheel_pos * 3, 0, wheel_pos * 3).into();
    }
    if wheel_pos < 170 {
        wheel_pos -= 85;
        return (0, wheel_pos * 3, 255 - wheel_pos * 3).into();
    }
    wheel_pos -= 170;
    (wheel_pos * 3, 255 - wheel_pos * 3, 0).into()
}

pub trait LEDPatternFragment {
    fn duration_ms(&self) -> u32;
    fn run(&self, t: u32, leds: &mut [RGB8; NUM_LEDS]);
}

#[derive(Clone, Debug)]
pub struct OneColor {
    pub duration: u32,
    pub color: RGB8,
}

impl LEDPatternFragment for OneColor {
    fn duration_ms(&self) -> u32 {
        self.duration
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for led in leds.iter_mut() {
            *led = self.color;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Off {
    pub duration: u32,
}

impl LEDPatternFragment for Off {
    fn duration_ms(&self) -> u32 {
        self.duration
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for led in leds.iter_mut() {
            *led = RGB8::default();
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoyalRainbow {
    pub duration: u32,
    pub direction: bool,
}
impl RoyalRainbow {
    pub fn new(duration: u32, direction: bool) -> Self {
        Self {
            duration,
            direction,
        }
    }
}
impl LEDPatternFragment for RoyalRainbow {
    fn duration_ms(&self) -> u32 {
        self.duration
    }

    fn run(&self, t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        let ti: i32 = t as i32;
        let td = if self.direction {-ti} else {ti};
        let j = td / 2;
        for (i, led) in leds.iter_mut().enumerate() {
            *led = wheel((((i * 256) as i32 / NUM_LEDS as i32 + j) & 255) as u8);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Colors {
    pub colors: [RGB8; NUM_LEDS],
}
impl LEDPatternFragment for Colors {
    fn duration_ms(&self) -> u32 {
        0
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for (i, led) in leds.iter_mut().enumerate() {
            *led = self.colors[i % NUM_LEDS];
        }
    }
}

trait LEDPatternFragmentDebug: LEDPatternFragment + fmt::Debug {}
impl<T: LEDPatternFragment + fmt::Debug> LEDPatternFragmentDebug for T {}

type FragmentVec = Vec<Box<dyn LEDPatternFragmentDebug>>;

#[derive(Clone, Debug)]
struct LEDPattern<'a> {
    fragments: &'a FragmentVec,
    current_fragment_idx: usize,
    current_fragment_start_ms: u64,
}
impl<'a> LEDPattern<'a> {
    fn new(fragments: &'a FragmentVec) -> Self {
            Self {
                fragments,
                current_fragment_idx: 0,
                current_fragment_start_ms: 0,
            }
        }

    fn update(&mut self, data: &mut [RGB8; NUM_LEDS], oneshot: bool) -> bool {
        if self.current_fragment_start_ms == 0 {
            self.current_fragment_start_ms = Instant::now().as_millis();
        }

        if self.fragments.is_empty() {
            return !oneshot;
        }
        let mut current_fragment_duration_ms: u32 =
            self.fragments[self.current_fragment_idx].duration_ms();

        let now_ms = Instant::now().as_millis();

        while now_ms - self.current_fragment_start_ms > current_fragment_duration_ms as u64 {
            self.current_fragment_idx += 1;
            if self.current_fragment_idx >= self.fragments.len() {
                if oneshot {
                    return false;
                }
                self.current_fragment_idx = 0;
            }
            self.current_fragment_start_ms += current_fragment_duration_ms as u64;
            current_fragment_duration_ms = self.fragments[self.current_fragment_idx].duration_ms();
        }

        // Call the run method of the current fragment
        let time_diff = (Instant::now().as_millis() - self.current_fragment_start_ms) as u32;
        self.fragments[self.current_fragment_idx].run(time_diff, data);

        true
    }
}

type ModifierVec<'a> = Vec<LEDPattern<'a>>;

struct LEDBlinker<'d, P: Instance, const S: usize> {
    ws2812: PioWs2812<'d, P, S, NUM_LEDS>,
    data: [RGB8; NUM_LEDS],
    last_colors: [RGB8; NUM_LEDS],
    pattern: LEDPattern<'d>,
    modifiers: ModifierVec<'d>,
    brightness: u8,
}

impl<'d, P: Instance, const S: usize> LEDBlinker<'d, P, S> {
    fn new(
        ws2812: PioWs2812<'d, P, S, NUM_LEDS>,
        pattern: LEDPattern<'d>,
        brightness: u8,
    ) -> Self {
        Self {
            ws2812,
            data: [RGB8::default(); NUM_LEDS],
            last_colors: [RGB8::default(); NUM_LEDS],
            pattern,
            modifiers: Vec::new(),
            brightness,
        }
    }

    fn set_pattern(&mut self, pattern: &'d LEDPattern) {
        self.pattern = pattern.clone();
    }

    fn add_modifier(&mut self, modifier: &'d LEDPattern) {
        self.modifiers.push(modifier.clone());
    }

    async fn update(&mut self) {
        self.data.copy_from_slice(&self.last_colors);
        self.pattern.update(&mut self.data, false);
        self.last_colors = self.data;

        let mut mods: ModifierVec<'d> = Vec::new();
        for modifier in self.modifiers.iter_mut() {
            if modifier.update(&mut self.data, true) {
                mods.push(modifier.clone());
            }
        }
        self.modifiers = mods;

        // Apply brightness
        for led in self.data.iter_mut() {
            *led = RGB8::new(
                (led.r as u16 * self.brightness as u16 / 255) as u8,
                (led.g as u16 * self.brightness as u16 / 255) as u8,
                (led.b as u16 * self.brightness as u16 / 255) as u8,
            );
        }
        self.ws2812.write(&self.data).await;
    }

    fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
    }

    fn get_brightness(&self) -> u8 {
        self.brightness
    }
}

#[task]
pub async fn led_blinker_task(r: RGBLEDResources) {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.pio, Irqs);

    let dma_ch0 = r.dma_ch;
    let rgb_led_pin = r.pin;

    let program = PioWs2812Program::new(&mut common);
    let ws2812 = PioWs2812::new(&mut common, sm0, dma_ch0, rgb_led_pin, &program);

    let fragments: FragmentVec = vec![
        Box::new(RoyalRainbow::new(1280, true)),
        Box::new(OneColor { duration: 1000, color: RGB8::new(255, 0, 0) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(0, 255, 0) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(0, 0, 255) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(255, 255, 0) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(255, 0, 255) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(0, 255, 255) }),
        Box::new(OneColor { duration: 1000, color: RGB8::new(255, 255, 255) }),
        Box::new(Off { duration: 1000 }),
    ];
    let pattern = LEDPattern::new(&fragments);

    let mut led_blinker = LEDBlinker::new(ws2812, pattern, 255);

    let mut ticker = Ticker::every(Duration::from_millis(10));

    loop {
        ticker.next().await;
        led_blinker.update().await;
    }
}
