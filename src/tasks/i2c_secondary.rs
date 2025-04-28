use embassy_executor::task;
use embassy_time::{Duration, Timer};

use crate::config_resources::I2CSecondaryResources;

#[task]
pub async fn i2c_secondary_task(r: I2CSecondaryResources) {
    loop {
        // Handle I2C secondary (slave) communication
        Timer::after(Duration::from_millis(100)).await;
    }
}
