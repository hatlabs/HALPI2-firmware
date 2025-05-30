use defmt::{error, info};
use embassy_time::{Duration, Timer};

use crate::{config, MFlashType};




#[embassy_executor::task]
pub async fn mark_firmware_booted_task(flash: &'static MFlashType<'static>) {
    // Wait for 30 seconds to ensure the firmware is stable and then
    // mark it as booted, preventing the bootloader from reverting
    // to the previous firmware on the next boot.
    Timer::after(Duration::from_millis(
        config::FIRMWARE_MARK_BOOTED_DELAY_MS as u64,
    ))
    .await;

    let config = embassy_boot_rp::FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut aligned = embassy_boot_rp::AlignedBuffer([0; 4]);
    let mut updater = embassy_boot_rp::FirmwareUpdater::new(config, &mut aligned.0);

    let bootloader_state_result = updater.get_state().await;
    let bootloader_state = match bootloader_state_result {
        Ok(state) => state,
        Err(e) => {
            error!(
                "Failed to get bootloader state: {:?}",
                defmt::Debug2Format(&e)
            );
            return;
        }
    };
    if bootloader_state == embassy_boot_rp::State::Swap {
        info!("Writing bootloader state to flash");
        let mark_result = updater.mark_booted().await;
        if let Err(e) = mark_result {
            error!(
                "Failed to mark firmware as booted: {:?}",
                defmt::Debug2Format(&e)
            );
        } else {
            info!("Firmware marked as booted successfully");
        }
    }
}
