#![no_std]
#![no_main]

extern crate alloc;

use config::FLASH_SIZE;
use embassy_rp::flash::Async;
use embassy_rp::watchdog::Watchdog;

use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex, once_lock::OnceLock};
use embassy_time::Duration;
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();
const HEAP_SIZE: usize = 65536; // 64kB

use defmt::{error, info};
use embassy_executor::Spawner;

use {defmt_rtt as _, panic_probe as _};

mod config;
mod config_resources;
mod flash_layout;
mod led_patterns;
mod tasks;

use crate::config_resources::{
    AnalogInputResources, AssignedResources, ConfigManagerOutputResources, DigitalInputResources, I2CPeripheralsResources,
    I2CSecondaryResources, PowerButtonInputResources, PowerButtonResources, RGBLEDResources,
    StateMachineOutputResources, TestModeResources, UserButtonInputResources,
};

pub type FlashType<'a> =
    embassy_rp::flash::Flash<'a, embassy_rp::peripherals::FLASH, Async, FLASH_SIZE>;
pub type MFlashType<'a> = Mutex<NoopRawMutex, FlashType<'a>>;

static OM_FLASH: OnceLock<MFlashType<'static>> = OnceLock::new();

static OM_WATCHDOG: OnceLock<Mutex<NoopRawMutex, Watchdog>> = OnceLock::new();

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

    // Override bootloader watchdog
    let watchdog_peripheral = p.WATCHDOG;
    let mut watchdog = Watchdog::new(watchdog_peripheral);
    watchdog.start(Duration::from_secs(8));
    match OM_WATCHDOG.init(Mutex::<NoopRawMutex, _>::new(watchdog)) {
        Ok(_) => info!("Watchdog initialized successfully"),
        Err(_) => error!("Failed to initialize watchdog"),
    }

    let flash = embassy_rp::flash::Flash::<embassy_rp::peripherals::FLASH, Async, FLASH_SIZE>::new(
        p.FLASH, p.DMA_CH1,
    );
    let flash: MFlashType = Mutex::<NoopRawMutex, _>::new(flash);

    match OM_FLASH.init(flash) {
        Ok(_) => info!("Flash initialized successfully"),
        Err(_) => error!("Failed to initialize flash"),
    }
    let flash = OM_FLASH.get().await;

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
        ))
        .unwrap();

    spawner
        .spawn(tasks::gpio_input::user_button_input_task(
            r.user_button_input,
        ))
        .unwrap();

    spawner
        .spawn(tasks::gpio_input::test_mode_input_task(r.test_mode))
        .unwrap();

    spawner
        .spawn(tasks::power_button::power_button_output_task(
            r.power_button,
        ))
        .unwrap();

    spawner
        .spawn(tasks::i2c_secondary::i2c_secondary_task(r.i2cs))
        .unwrap();

    spawner
        .spawn(tasks::state_machine::state_machine_task(
            r.state_machine_outputs,
        ))
        .unwrap();

    spawner
        .spawn(tasks::led_blinker::led_blinker_task(r.rgb_led))
        .unwrap();

    spawner
        .spawn(tasks::i2c_peripheral::i2c_peripheral_access_task(r.i2cm))
        .unwrap();

    spawner
        .spawn(tasks::watchdog_feeder::watchdog_feeder_task())
        .unwrap();

    spawner
        .spawn(tasks::flash_writer::flash_writer_task(flash))
        .unwrap();

    spawner
        .spawn(tasks::mark_firmware_booted::mark_firmware_booted_task(
            flash,
        ))
        .unwrap();

    spawner
        .spawn(tasks::config_manager::config_manager_task(flash, r.config_manager_outputs))
        .unwrap();
}
