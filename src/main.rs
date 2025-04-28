#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

mod config_resources;
mod tasks;

use crate::config_resources::{
    AssignedResources, DigitalInputResources, I2CPeripheralsResources, I2CSecondaryResources,
    RGBLEDResources, AnalogInputResources, StateMachineOutputResources, TestModeResources,
};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let r = split_resources!(p);

    info!("Starting up...");

    // Spawn the async tasks
    // spawner
        // .spawn(tasks::gpio_input::digital_input_task(r.digital_inputs))
        // .unwrap();
    // spawner
        // .spawn(tasks::gpio_input::analog_input_task(r.analog_inputs))
        // .unwrap();

    //spawner
    //    .spawn(tasks::i2c_secondary::i2c_secondary_task(r.i2cs))
    //    .unwrap();
    //spawner
    //    .spawn(tasks::state_machine::state_machine_task(
    //        r.state_machine_outputs,
    //    ))
    //    .unwrap();
    spawner
        .spawn(tasks::led_blinker::led_blinker_task(r.rgb_led))
        .unwrap();
    //spawner
    //    .spawn(tasks::i2c_peripheral::i2c_peripheral_access_task(r.i2cm))
    //    .unwrap();

    // Main task can handle other initialization or remain idle
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
