use defmt::error;
use embassy_executor::task;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::InterruptHandler;
use embassy_rp::peripherals::I2C0;
use embassy_time::{Duration, Timer};

use crate::config_resources::I2CPeripheralsResources;
use crate::tasks::gpio_input::INPUTS;

const TMP112_ADDR: u8 = 0x4b; // TMP112 I2C address

bind_interrupts!(struct Irqs {
    I2C0_IRQ => InterruptHandler<I2C0>;
});

#[task]
pub async fn i2c_peripheral_access_task(r: I2CPeripheralsResources) {
    let config = embassy_rp::i2c::Config::default();
    let mut bus = embassy_rp::i2c::I2c::new_async(r.i2c, r.scl, r.sda, Irqs, config);

    loop {
        // Handle I2C peripheral communication
        Timer::after(Duration::from_millis(1000)).await;

        let mut response = [0u8; 2];
        let result = bus
            .write_read_async(TMP112_ADDR, [0x00u8], &mut response)
            .await;
        if let Err(e) = result {
            error!("I2C write_read failed: {:?}", e);
            continue;
        }
        // Process the received data
        let temperature = ((response[0] as u16) << 8) | (response[1] as u16);
        let celsius = (temperature >> 4) as f32 * 0.0625;
        let kelvin = celsius + 273.15;
        let mut inputs = INPUTS.lock().await;
        inputs.pcb_temp = kelvin;
    }
}
