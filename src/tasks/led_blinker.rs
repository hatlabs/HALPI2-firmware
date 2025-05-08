use core::fmt;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use embassy_executor::task;
use embassy_rp::bind_interrupts;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{self, Receiver};
use embassy_time::{Duration, Instant, Ticker};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{Instance, InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use smart_leds::{brightness, gamma, RGB8};

use crate::config_resources::RGBLEDResources;

const NUM_LEDS: usize = 5;

pub enum LEDBlinkerEvents {
    SetPattern(LEDPattern),
    SetBrightness(u8),
    AddModifier(LEDPattern),
}

pub type LEDBlinkerChannelType = channel::Channel<CriticalSectionRawMutex, LEDBlinkerEvents, 8>;
pub static LED_BLINKER_EVENT_CHANNEL: LEDBlinkerChannelType = channel::Channel::new();

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
    pub duration_ms: u32,
    pub color: RGB8,
}

impl OneColor {
    pub fn new(duration_ms: u32, color: RGB8) -> Self {
        Self { duration_ms, color }
    }
}

impl LEDPatternFragment for OneColor {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for led in leds.iter_mut() {
            *led = self.color;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Off {
    pub duration_ms: u32,
}

impl Off {
    pub fn new(duration: u32) -> Self {
        Self { duration_ms: duration }
    }
}

impl LEDPatternFragment for Off {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for led in leds.iter_mut() {
            *led = RGB8::default();
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoyalRainbow {
    pub duration_ms: u32,
    pub direction: bool,
}
impl RoyalRainbow {
    pub fn new(duration: u32, direction: bool) -> Self {
        Self {
            duration_ms: duration,
            direction,
        }
    }
}
impl LEDPatternFragment for RoyalRainbow {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run(&self, t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        let ti: i32 = t as i32;
        let td = if self.direction { -ti } else { ti };
        let j = td / 2;
        for (i, led) in leds.iter_mut().enumerate() {
            *led = wheel((((i * 256) as i32 / NUM_LEDS as i32 + j) & 255) as u8);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Colors {
    pub duration_ms: u32,
    pub colors: [RGB8; NUM_LEDS],
}

impl Colors {
    pub fn new(duration_ms: u32, colors: [RGB8; NUM_LEDS]) -> Self {
        Self { duration_ms, colors }
    }
}

impl LEDPatternFragment for Colors {
    fn duration_ms(&self) -> u32 {
        1000
    }

    fn run(&self, _t: u32, leds: &mut [RGB8; NUM_LEDS]) {
        for (i, led) in leds.iter_mut().enumerate() {
            *led = self.colors[i % NUM_LEDS];
        }
    }
}

pub trait LEDPatternFragmentDebug: LEDPatternFragment + fmt::Debug + Send {}
impl<T: LEDPatternFragment + fmt::Debug + Send> LEDPatternFragmentDebug for T {}

pub type FragmentVec = Vec<Box<dyn LEDPatternFragmentDebug>>;

#[derive(Debug)]
pub struct LEDPattern {
    fragments: FragmentVec,
    current_fragment_idx: usize,
    current_fragment_start_ms: u64,
}
impl LEDPattern {
    pub fn new(fragments: FragmentVec) -> Self {
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

type ModifierVec = Vec<LEDPattern>;

struct LEDBlinker<'d, P: Instance, const S: usize> {
    ws2812: PioWs2812<'d, P, S, NUM_LEDS>,
    data: [RGB8; NUM_LEDS],
    last_colors: [RGB8; NUM_LEDS],
    pattern: LEDPattern,
    modifiers: ModifierVec,
    brightness: u8,
}

impl<'d, P: Instance, const S: usize> LEDBlinker<'d, P, S> {
    fn new(ws2812: PioWs2812<'d, P, S, NUM_LEDS>, pattern: LEDPattern, brightness: u8) -> Self {
        Self {
            ws2812,
            data: [RGB8::default(); NUM_LEDS],
            last_colors: [RGB8::default(); NUM_LEDS],
            pattern,
            modifiers: Vec::new(),
            brightness,
        }
    }

    fn set_pattern(&mut self, pattern: LEDPattern) {
        self.pattern = pattern;
    }

    fn add_modifier(&mut self, modifier: LEDPattern) {
        self.modifiers.push(modifier);
    }

    async fn update(&mut self) {
        self.data.copy_from_slice(&self.last_colors);
        self.pattern.update(&mut self.data, false);
        self.last_colors = self.data;

        let mut mods: ModifierVec = Vec::new();
        for (i, modifier) in self.modifiers.iter_mut().enumerate() {
            if !modifier.update(&mut self.data, true) {
                mods.remove(i);
            }
        }
        self.modifiers = mods;

        // Apply brightness

        //for led in self.data.iter_mut() {
        //    *led = RGB8::new(
        //        (led.r as u16 * self.brightness as u16 / 255) as u8,
        //        (led.g as u16 * self.brightness as u16 / 255) as u8,
        //        (led.b as u16 * self.brightness as u16 / 255) as u8,
        //    );
        //}

        let gamma_corrected = gamma(self.data.iter().cloned());
        let brightness_corrected = brightness(gamma_corrected, self.brightness);
        let corrected_data = brightness_corrected
            .map(|color| RGB8::new(color.r, color.g, color.b));
        // Write the data back into an RGB8 array
        let mut output_data: [RGB8; NUM_LEDS] = [RGB8::default(); NUM_LEDS];
        for (i, color) in corrected_data.enumerate() {
            output_data[i] = color;
        }

        self.ws2812.write(&output_data).await;
    }

    fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
    }

    fn get_brightness(&self) -> u8 {
        self.brightness
    }
}

#[task]
pub async fn led_blinker_task(r: RGBLEDResources, channel: &'static LEDBlinkerChannelType) {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.pio, Irqs);

    bind_interrupts!(struct Irqs {
        PIO0_IRQ_0 => InterruptHandler<PIO0>;
    });

    let dma_ch0 = r.dma_ch;
    let rgb_led_pin = r.pin;

    let program = PioWs2812Program::new(&mut common);
    let ws2812 = PioWs2812::new(&mut common, sm0, dma_ch0, rgb_led_pin, &program);

    let fragments: FragmentVec = vec![Box::new(Off { duration_ms: 1000 })];
    let pattern = LEDPattern::new(fragments);

    let mut led_blinker = LEDBlinker::new(ws2812, pattern, 255);

    let mut ticker = Ticker::every(Duration::from_millis(10));

    let receiver = channel.receiver();
    loop {
        if !(receiver.is_empty()) {
            let event = receiver.receive().await;

            match event {
                LEDBlinkerEvents::SetPattern(pattern) => led_blinker.set_pattern(pattern),
                LEDBlinkerEvents::SetBrightness(brightness) => {
                    led_blinker.set_brightness(brightness)
                }
                LEDBlinkerEvents::AddModifier(modifier) => led_blinker.add_modifier(modifier),
            }
        }

        ticker.next().await;
        led_blinker.update().await;
    }
}
