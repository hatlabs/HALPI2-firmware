use core::fmt;
use core::future::Future;
use core::pin::Pin;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use defmt::{debug, info};
use embassy_executor::task;
use embassy_rp::bind_interrupts;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel;
use embassy_time::{Duration, Instant, Ticker};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{Instance, InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use smart_leds::{RGB8, brightness, gamma};

use crate::config_resources::RGBLEDResources;
use crate::tasks::gpio_input::INPUTS;

const NUM_LEDS: usize = 5;

#[allow(dead_code)]
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

// Object-safe trait using boxed futures
pub trait LEDPatternFragment: Send {
    fn duration_ms(&self) -> u32;
    fn run<'a>(&'a self, t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>>;
    fn type_name(&self) -> &'static str;
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

    fn run<'a>(&'a self, _t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>> {
        Box::pin(async move {
            for led in leds.iter_mut() {
                *led = self.color;
            }
        })
    }

    fn type_name(&self) -> &'static str {
        "OneColor"
    }
}

#[derive(Clone, Debug)]
pub struct Off {
    pub duration_ms: u32,
}

impl Off {
    pub fn new(duration: u32) -> Self {
        Self {
            duration_ms: duration,
        }
    }
}

impl LEDPatternFragment for Off {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run<'a>(&'a self, _t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>> {
        Box::pin(async move {
            for led in leds.iter_mut() {
                *led = RGB8::default();
            }
        })
    }

    fn type_name(&self) -> &'static str {
        "Off"
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

    fn run<'a>(&'a self, t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>> {
        Box::pin(async move {
            let ti: i32 = t as i32;
            let td = if self.direction { -ti } else { ti };
            let j = td / 2;
            for (i, led) in leds.iter_mut().enumerate() {
                *led = wheel((((i * 256) as i32 / NUM_LEDS as i32 + j) & 255) as u8);
            }
        })
    }

    fn type_name(&self) -> &'static str {
        "RoyalRainbow"
    }
}

#[derive(Clone, Debug)]
pub struct Colors {
    pub duration_ms: u32,
    pub colors: [RGB8; NUM_LEDS],
}

impl Colors {
    pub fn new(duration_ms: u32, colors: [RGB8; NUM_LEDS]) -> Self {
        Self {
            duration_ms,
            colors,
        }
    }
}

impl LEDPatternFragment for Colors {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run<'a>(&'a self, _t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>> {
        Box::pin(async move {
            for (i, led) in leds.iter_mut().enumerate() {
                *led = self.colors[i % NUM_LEDS];
            }
        })
    }

    fn type_name(&self) -> &'static str {
        "Colors"
    }
}

pub struct SupercapBar {
    pub duration_ms: u32,
    pub color: RGB8,
}

impl SupercapBar {
    pub fn new(duration_ms: u32, color: RGB8) -> Self {
        Self { duration_ms, color }
    }
}

impl LEDPatternFragment for SupercapBar {
    fn duration_ms(&self) -> u32 {
        self.duration_ms
    }

    fn run<'a>(&'a self, _t: u32, leds: &'a mut [RGB8; NUM_LEDS]) -> Pin<Box<dyn Future<Output = ()> + 'a + Send>> {
        let color = self.color;
        Box::pin(async move {
            // Read the current supercap voltage (in V)
            let vscap = {
                let inputs = INPUTS.lock().await;
                inputs.vscap
            };

            for (i, led) in leds.iter_mut().enumerate().take(NUM_LEDS) {
                let low = 5.0 + i as f32;
                let high = 6.0 + i as f32;
                if vscap >= high {
                    *led = color;
                } else if vscap > low {
                    let frac = (vscap - low).clamp(0.0, 1.0);
                    *led = RGB8 {
                        r: (color.r as f32 * frac) as u8,
                        g: (color.g as f32 * frac) as u8,
                        b: (color.b as f32 * frac) as u8,
                    };
                } else {
                    *led = RGB8::default();
                }
            }
        })
    }

    fn type_name(&self) -> &'static str {
        "SupercapBar"
    }
}

pub type FragmentVec = Vec<Box<dyn LEDPatternFragment>>;

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

    async fn update(&mut self, data: &mut [RGB8; NUM_LEDS], oneshot: bool) -> bool {
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
        self.fragments[self.current_fragment_idx].run(time_diff, data).await;

        true
    }
}

// Implement Debug manually for LEDPattern
impl fmt::Debug for LEDPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fragment_types: Vec<&str> = self.fragments.iter().map(|f| f.type_name()).collect();
        f.debug_struct("LEDPattern")
            .field("current_fragment_idx", &self.current_fragment_idx)
            .field("current_fragment_start_ms", &self.current_fragment_start_ms)
            .field("fragments_count", &self.fragments.len())
            .field("fragment_types", &fragment_types)
            .finish()
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
        self.pattern.update(&mut self.data, false).await;
        self.last_colors = self.data;

        // Use retain instead of manual removal to avoid index issues
        let mut i = 0;
        while i < self.modifiers.len() {
            if !self.modifiers[i].update(&mut self.data, true).await {
                self.modifiers.remove(i);
            } else {
                i += 1;
            }
        }

        // Apply brightness
        let gamma_corrected = gamma(self.data.iter().cloned());
        let brightness_corrected = brightness(gamma_corrected, self.brightness);
        let corrected_data: Vec<RGB8> = brightness_corrected.collect();

        // Write the data back into an RGB8 array
        let mut output_data: [RGB8; NUM_LEDS] = [RGB8::default(); NUM_LEDS];
        for (i, color) in corrected_data.into_iter().enumerate().take(NUM_LEDS) {
            output_data[i] = color;
        }

        self.ws2812.write(&output_data).await;
    }

    fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
    }

    #[allow(dead_code)]
    fn get_brightness(&self) -> u8 {
        self.brightness
    }
}

pub async fn set_led_brightness(brightness: u8) {
    // Save the brightness to flash using the config manager
    crate::tasks::config_manager::set_led_brightness(brightness).await;

    LED_BLINKER_EVENT_CHANNEL
        .send(LEDBlinkerEvents::SetBrightness(brightness))
        .await;
    info!("LED brightness set to {}", brightness);
}

pub async fn get_led_brightness() -> u8 {
    crate::tasks::config_manager::get_led_brightness().await
}

#[task]
pub async fn led_blinker_task(r: RGBLEDResources) {
    info!("Initializing LED blinker task");
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

    // Hackety hack: create a 100 ms delay to allow the config manager to initialize
    embassy_time::Timer::after(Duration::from_millis(100)).await;

    debug!("Getting LED brightness from config");
    let brightness = crate::tasks::config_manager::get_led_brightness().await;
    debug!("LED brightness from config: {}", brightness);

    let mut led_blinker = LEDBlinker::new(ws2812, pattern, brightness);

    let mut ticker = Ticker::every(Duration::from_millis(10));

    let receiver = LED_BLINKER_EVENT_CHANNEL.receiver();

    info!("LED blinker task initialized");

    loop {
        if !(receiver.is_empty()) {
            let event = receiver.receive().await;

            match event {
                LEDBlinkerEvents::SetPattern(pattern) => led_blinker.set_pattern(pattern),
                LEDBlinkerEvents::SetBrightness(brightness) => {
                    led_blinker.set_brightness(brightness);
                }
                LEDBlinkerEvents::AddModifier(modifier) => led_blinker.add_modifier(modifier),
            }
        }

        ticker.next().await;
        led_blinker.update().await;
    }
}
