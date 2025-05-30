use defmt::debug;
use embassy_time::{Duration, Timer};

use crate::{OM_WATCHDOG, tasks};

#[embassy_executor::task]
pub async fn watchdog_feeder_task() {
    // This task feeds the watchdog to prevent system reset

    loop {
        // Feed the watchdog every 1 second
        Timer::after(Duration::from_secs(1)).await;
        OM_WATCHDOG.get().await.lock().await.feed(); // Feed the watchdog

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
