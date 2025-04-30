use core::fmt;

use defmt::debug;
use embassy_executor::task;
use embassy_rp::bind_interrupts;
use heapless::Vec;

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
const FRAGMENT_VEC_SIZE: usize = 30;
const MODIFIER_VEC_SIZE: usize = 5;

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
}
impl LEDPatternFragment for RoyalRainbow {
    fn duration_ms(&self) -> u32 {
        self.duration
    }

    fn run(&self, t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        let j = t / 2;
        for (i, led) in leds.iter_mut().enumerate() {
            *led = wheel((((i * 256) as u16 / NUM_LEDS as u16 + j as u16) & 255) as u8);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Colors {
    pub colors: Vec<RGB8, NUM_LEDS>,
}
impl LEDPatternFragment for Colors {
    fn duration_ms(&self) -> u32 {
        0
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for (i, led) in leds.iter_mut().enumerate() {
            *led = self.colors[i % self.colors.len()];
        }
    }
}

trait LEDPatternFragmentDebug: LEDPatternFragment + fmt::Debug {}
impl<T: LEDPatternFragment + fmt::Debug> LEDPatternFragmentDebug for T {}

type FragmentVec = Vec<&'static dyn LEDPatternFragmentDebug, FRAGMENT_VEC_SIZE>;

#[derive(Clone, Debug)]
struct LEDPattern {
    fragments: FragmentVec,
    current_fragment_idx: usize,
    current_fragment_start_ms: u64,
}
impl LEDPattern {
    fn new(fragments: FragmentVec) -> Self {
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

type ModifierVec = Vec<LEDPattern, MODIFIER_VEC_SIZE>;

struct LEDBlinker<'d, P: Instance, const S: usize> {
    ws2812: PioWs2812<'d, P, S, NUM_LEDS>,
    data: &'d mut [RGB8; NUM_LEDS],
    last_colors: [RGB8; NUM_LEDS],
    pattern: LEDPattern,
    modifiers: ModifierVec,
}

impl<'d, P: Instance, const S: usize> LEDBlinker<'d, P, S> {
    fn new(
        ws2812: PioWs2812<'d, P, S, NUM_LEDS>,
        data: &'d mut [RGB8; NUM_LEDS],
        pattern: LEDPattern,
    ) -> Self {
        Self {
            ws2812,
            data,
            last_colors: [RGB8::default(); NUM_LEDS],
            pattern,
            modifiers: Vec::new(),
        }
    }

    fn set_pattern(&mut self, pattern: &LEDPattern) {
        self.pattern = pattern.clone();
    }

    fn add_modifier(&mut self, modifier: &LEDPattern) {
        self.modifiers.push(modifier.clone()).unwrap();
    }

    async fn update(&mut self) {
        self.pattern.update(self.data, false);

        let mut mods: ModifierVec = Vec::new();
        for modifier in self.modifiers.iter_mut() {
            if modifier.update(self.data, true) {
                mods.push(modifier.clone()).unwrap();
            }
        }
        self.modifiers = mods;

        self.ws2812.write(self.data).await;
    }
}

#[task]
pub async fn led_blinker_task(r: RGBLEDResources) {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.pio, Irqs);

    let dma_ch0 = r.dma_ch;
    let rgb_led_pin = r.pin;

    let mut data = [RGB8::default(); NUM_LEDS];

    let program = PioWs2812Program::new(&mut common);
    let ws2812 = PioWs2812::new(&mut common, sm0, dma_ch0, rgb_led_pin, &program);

    let fragments: FragmentVec =
        Vec::from_slice(&[&(RoyalRainbow { duration: 2560 }) as &dyn LEDPatternFragmentDebug])
            .unwrap();
    let pattern = LEDPattern::new(fragments);

    let mut led_blinker = LEDBlinker::new(ws2812, &mut data, pattern);

    let mut ticker = Ticker::every(Duration::from_millis(10));

    loop {
        ticker.next().await;
        led_blinker.update().await;
    }
}
