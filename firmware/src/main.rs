#![no_std]
#![no_main]

extern crate alloc;

use config::FLASH_SIZE;
use embassy_rp::{flash::{Async}, watchdog::Watchdog};
use embassy_sync::{
    blocking_mutex::raw::{NoopRawMutex},
    mutex::Mutex,
    once_lock::OnceLock,
};
use embedded_alloc::LlffHeap as Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();
const HEAP_SIZE: usize = 65536; // 64kB

use config_manager::init_config_manager;
use defmt::{debug, error, info};
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

mod config;
mod config_manager;
mod config_resources;
mod flash_layout;
mod led_patterns;
mod tasks;

use crate::config_resources::{
    AnalogInputResources, AssignedResources, DigitalInputResources, I2CPeripheralsResources,
    I2CSecondaryResources, PowerButtonInputResources, PowerButtonResources, RGBLEDResources,
    StateMachineOutputResources, TestModeResources, UserButtonInputResources,
};

pub type FlashType<'a> =
    embassy_rp::flash::Flash<'a, embassy_rp::peripherals::FLASH, Async, FLASH_SIZE>;
pub type MFlashType<'a> = Mutex<NoopRawMutex, FlashType<'a>>;
pub static OM_FLASH: OnceLock<MFlashType<'static>> = OnceLock::new();

#[embassy_executor::task]
async fn mark_firmware_booted_task() {
    // Wait for 30 seconds to ensure the firmware is stable and then
    // mark it as booted, preventing the bootloader from reverting
    // to the previous firmware on the next boot.
    Timer::after(Duration::from_millis(config::FIRMWARE_MARK_BOOTED_DELAY_MS as u64)).await;

    let flash = OM_FLASH.get().await;
    let config = embassy_boot::FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut aligned = embassy_boot::AlignedBuffer([0; 1]);
    let mut updater = embassy_boot::FirmwareUpdater::new(config, &mut aligned.0);

    let bootloader_state = updater.get_state().await.unwrap();
    if bootloader_state == embassy_boot::State::Swap {
        info!("Marking firmware as booted");
        updater.mark_booted().await.unwrap();
    }
}

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
    let mut watchdog = Watchdog::new(p.WATCHDOG);
    watchdog.start(Duration::from_secs(8));

    // Initialize the config manager
    let flash = embassy_rp::flash::Flash::<embassy_rp::peripherals::FLASH, Async, FLASH_SIZE>::new(
        p.FLASH, p.DMA_CH1,
    );
    let flash: MFlashType = Mutex::<NoopRawMutex, _>::new(flash);

    if OM_FLASH.init(flash).is_err() {
        error!("Failed to initialize flash");
        return;
    }

    info!("Initializing config manager...");

    init_config_manager().await;

    info!("Config manager initialized.");

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
        .spawn(tasks::power_button::power_button_output_task(
            r.power_button,
        ))
        .unwrap();

    spawner
        .spawn(tasks::host_watchdog::host_watchdog_task())
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
        .spawn(tasks::flash_writer::flash_writer_task())
        .unwrap();

    //spawner
    //    .spawn(tasks::i2c_peripheral::i2c_peripheral_access_task(r.i2cm))
    //    .unwrap();

    spawner
        .spawn(mark_firmware_booted_task())
        .unwrap();

    // Main task can handle other initialization or remain idle
    loop {
        Timer::after(Duration::from_secs(1)).await;

        watchdog.feed();

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
