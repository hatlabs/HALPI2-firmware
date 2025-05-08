#![no_std]
#![no_main]

extern crate alloc;

use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();
const HEAP_SIZE: usize = 65536; // 64kB

use defmt::{debug, info};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use tasks::led_blinker::LED_BLINKER_EVENT_CHANNEL;
use tasks::power_button::POWER_BUTTON_EVENT_CHANNEL;
use {defmt_rtt as _, panic_probe as _};

mod config;
mod config_resources;
mod led_patterns;
mod tasks;

use crate::config_resources::{
    AnalogInputResources, AssignedResources, DigitalInputResources, I2CPeripheralsResources,
    I2CSecondaryResources, PowerButtonInputResources, PowerButtonResources, RGBLEDResources,
    StateMachineOutputResources, TestModeResources, UserButtonInputResources,
};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize the allocator BEFORE you use it
    {
        use core::mem::MaybeUninit;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(&raw mut HEAP_MEM as usize, HEAP_SIZE) }
    }

    let p = embassy_rp::init(Default::default());
    let r = split_resources!(p);

    info!("Starting up...");

    // Spawn the async tasks
    spawner
        .spawn(tasks::gpio_input::digital_input_task(r.digital_inputs))
        .unwrap();

    spawner
        .spawn(tasks::gpio_input::analog_input_task(r.analog_inputs))
        .unwrap();

    spawner
        .spawn(tasks::gpio_input::power_button_input_task(
            r.power_button_input,
            &POWER_BUTTON_EVENT_CHANNEL,
        ))
        .unwrap();

    spawner
        .spawn(tasks::gpio_input::user_button_input_task(
            r.user_button_input,
        ))
        .unwrap();

    spawner
        .spawn(tasks::power_button::power_button_output_task(
            r.power_button,
            &POWER_BUTTON_EVENT_CHANNEL,
        ))
        .unwrap();

    spawner
        .spawn(tasks::i2c_secondary::i2c_secondary_task(r.i2cs))
        .unwrap();

    spawner
        .spawn(tasks::state_machine::state_machine_task(
            r.state_machine_outputs,
            &POWER_BUTTON_EVENT_CHANNEL,
            &LED_BLINKER_EVENT_CHANNEL,
        ))
        .unwrap();

    spawner
        .spawn(tasks::led_blinker::led_blinker_task(
            r.rgb_led,
            &LED_BLINKER_EVENT_CHANNEL,
        ))
        .unwrap();

    //spawner
    //    .spawn(tasks::i2c_peripheral::i2c_peripheral_access_task(r.i2cm))
    //    .unwrap();

    // Main task can handle other initialization or remain idle
    loop {
        Timer::after(Duration::from_secs(1)).await;

        let inputs = tasks::gpio_input::INPUTS.lock().await;
        debug!(
            "vin: {:?} | vscap: {:?} | iin: {:?} | mcu_temp: {:?} | pcb_temp: {:?} | cm_on: {:?} | led_pwr: {:?} | led_active: {:?} | pg_5v: {:?} ",
            inputs.vin,
            inputs.vscap,
            inputs.iin,
            inputs.mcu_temp,
            inputs.pcb_temp,
            inputs.cm_on,
            inputs.led_pwr,
            inputs.led_active,
            inputs.pg_5v
        );
    }
}
