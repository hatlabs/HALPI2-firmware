#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

mod tasks;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let _p = embassy_rp::init(Default::default());

    // Spawn the async tasks
    spawner.spawn(tasks::i2c_slave::i2c_slave_task()).unwrap();
    spawner.spawn(tasks::state_machine::state_machine_task()).unwrap();
    spawner.spawn(tasks::led_blinker::led_blinker_task()).unwrap();
    spawner.spawn(tasks::i2c_peripheral::i2c_peripheral_access_task()).unwrap();

    // Main task can handle other initialization or remain idle
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
