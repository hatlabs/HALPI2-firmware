use defmt::*;
use embassy_executor::task;
use embassy_rp::bind_interrupts;

use embassy_time::{Duration, Ticker};

use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use smart_leds::RGB8;

use crate::config_resources::RGBLEDResources;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

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

#[task]
pub async fn led_blinker_task(r: RGBLEDResources) {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.pio, Irqs);

    let dma_ch0 = r.dma_ch;
    let rgb_led_pin = r.pin;

    const NUM_LEDS: usize = 5;

    let mut data = [RGB8::default(); NUM_LEDS];

    let program = PioWs2812Program::new(&mut common);
    let mut ws2812 = PioWs2812::new(&mut common, sm0, dma_ch0, rgb_led_pin, &program);

    let mut ticker = Ticker::every(Duration::from_millis(1000));

    loop {
        for j in 0..(256 * 5) {
            debug!("New Colors:");
            for (i, led) in data.iter_mut().enumerate() {
                *led = wheel((((i * 256) as u16 / NUM_LEDS as u16 + j as u16) & 255) as u8);
                debug!("R: {} G: {} B: {}", led.r, led.g, led.b);
            }
            ws2812.write(&data).await;

            ticker.next().await;
        }
    }
}
