// Flash updater task for handling firmware updates via I2C

use embassy_time::Duration;

use alloc::{string::String, vec::Vec};

use defmt::{debug, info, trace, warn};
use embassy_boot::{FirmwareState, State};
use embassy_boot_rp::{AlignedBuffer, FirmwareUpdater, FirmwareUpdaterConfig};
use embassy_executor::task;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel, mutex::Mutex};

use embassy_time::Timer;
// Import the NorFlash trait for async write support
use embedded_storage_async::nor_flash::{NorFlash, ReadNorFlash};

use crate::{
    MFlashType,
    config::{FLASH_ERASE_BLOCK_SIZE, FLASH_WRITE_BLOCK_SIZE, MAX_FLASH_WRITE_QUEUE_DEPTH},
};

#[derive(Clone, Copy, PartialEq)]
pub enum FlashUpdateState {
    Idle,
    Preparing,
    Updating,
    ReadyToCommit,
    WriteError,
    ProtocolError,
    Complete,
}

// Message passed from I2C task to Flash writer task
pub enum FlashWriteCommand {
    StartUpdate { total_size: u32 },
    WriteBlock { block_num: u16, data: Vec<u8> },
    Commit,
    Abort,
}

// Status shared between tasks
pub struct WriterStatus {
    pub state: FlashUpdateState,
    pub blocks_received: u16,
    pub offset: u32,
    pub total_size: u32,
    pub total_num_blocks: u16,
    pub error_details: Option<String>,
}

impl WriterStatus {
    const fn new() -> Self {
        Self {
            state: FlashUpdateState::Idle,
            blocks_received: 0,
            offset: 0,
            total_size: 0,
            total_num_blocks: 0,
            error_details: None,
        }
    }
}
// Channel for sending flash write requests from I2C task to Flash writer task
pub type FlashWriteRequestChannelType =
    channel::Channel<CriticalSectionRawMutex, FlashWriteCommand, MAX_FLASH_WRITE_QUEUE_DEPTH>;
pub static FLASH_WRITE_REQUEST_CHANNEL: FlashWriteRequestChannelType = channel::Channel::new();

// Shared status between tasks
pub static FLASH_WRITER_STATUS: Mutex<CriticalSectionRawMutex, WriterStatus> =
    Mutex::new(WriterStatus::new());

async fn prepare_update(flash: &MFlashType<'static>) -> Result<(), String> {
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    debug!("Got partition config for firmware updater");
    let mut aligned = embassy_boot_rp::AlignedBuffer([0; 4]);

    // FirmwareUpdater::prepare_update() blocks for a very long time, so
    // instead, implement it locally and erase the flash in smaller chunks.

    debug!("Preparing firmware update");
    let mut firmware_state = FirmwareState::new(config.state, &mut aligned.0);

    // Verify that we are in a booted firmware to avoid reverting to a bad state
    let state = match firmware_state.get_state().await {
        Ok(state) => state,
        Err(e) => {
            defmt::error!("E: {:?}", defmt::Debug2Format(&e));
            return Err("Failed to get firmware state".into());
        }
    };
    if !(state == State::Boot || state == State::DfuDetach || state == State::Revert || state == State::Swap) {
        return Err("Firmware is not in a booted state".into());
    }

    // Erase the flash partition

    let mut dfu_partition = config.dfu;

    // Erase the partition in FLASH_ERASE_BLOCK_SIZE sized chunks
    let mut offset = 0;
    let mut erase_size = dfu_partition.capacity();
    warn!("Erasing flash partition between 0 and {}", erase_size);
    while erase_size > 0 {
        // Allow other tasks to run
        Timer::after(Duration::from_micros(1)).await;

        let chunk_size = erase_size.min(FLASH_ERASE_BLOCK_SIZE) as u32;
        debug!(
            "Erasing flash chunk at offset {} with size {}",
            offset, chunk_size
        );

        match dfu_partition.erase(offset, offset + chunk_size).await {
            Ok(_) => {
                trace!("Flash chunk erased successfully");
                offset += chunk_size;
                erase_size -= chunk_size as usize;
            }
            Err(e) => {
                defmt::error!("E: {:?}", defmt::Debug2Format(&e));
                return Err("Flash erase failed".into());
            }
        }
    }
    info!("Flash partition erased successfully");
    Ok(())
}

async fn write_block(
    flash: &MFlashType<'static>,
    offset: u32,
    data: Vec<u8>,
) -> Result<(), String> {
    // Get the flash partition (implement this according to your OM_FLASH type)
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut writer = config.dfu;

    let mut buf: AlignedBuffer<FLASH_WRITE_BLOCK_SIZE> = AlignedBuffer([0; FLASH_WRITE_BLOCK_SIZE]);
    let len_data = data.len();

    buf.0[..len_data].copy_from_slice(&data);
    // Write the block to flash
    let write_result = writer.write(offset, &buf.0[..]).await;

    match write_result {
        Ok(_) => {
            defmt::debug!("block write successful");
            Ok(())
        }
        Err(e) => {
            defmt::warn!("E: {:?}", defmt::Debug2Format(&e));
            Err("Flash write failed".into())
        }
    }
}

async fn start_update(flash: &MFlashType<'static>, total_size: u32, num_blocks: u16) {
    // Initialize the flash writer status
    {
        let mut status = { FLASH_WRITER_STATUS.lock().await };
        status.state = FlashUpdateState::Preparing;
        status.blocks_received = 0;
        status.offset = 0;
        status.total_size = total_size;
        status.total_num_blocks = num_blocks;
        status.error_details = None;
    }
    match prepare_update(flash).await {
        Ok(_) => {
            defmt::debug!("update prepared");
            let mut status = { FLASH_WRITER_STATUS.lock().await };
            status.state = FlashUpdateState::Updating;
        }
        Err(e) => {
            defmt::error!("E: {:?}", defmt::Debug2Format(&e));
            let mut status = { FLASH_WRITER_STATUS.lock().await };
            status.state = FlashUpdateState::WriteError;
            status.error_details = Some(e);
        }
    }
}

async fn abort_update() {
    let mut status = { FLASH_WRITER_STATUS.lock().await };
    status.state = FlashUpdateState::Idle;
    status.blocks_received = 0;
    status.offset = 0;
    status.total_size = 0;
    status.total_num_blocks = 0;
    status.error_details = None;
}

async fn commit_update(flash: &MFlashType<'static>) {
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut aligned = embassy_boot_rp::AlignedBuffer([0; 4]);
    let mut firmware_updater = FirmwareUpdater::new(config, &mut aligned.0);
    match firmware_updater.mark_updated().await {
        Ok(_) => {
            defmt::debug!("update committed");
            let mut status = FLASH_WRITER_STATUS.lock().await;
            status.state = FlashUpdateState::Complete;
            status.blocks_received = 0;
            status.offset = 0;
            status.total_size = 0;
            status.total_num_blocks = 0;
            status.error_details = None;
        }
        Err(e) => {
            defmt::warn!("E: {:?}", defmt::Debug2Format(&e));
            let mut status = FLASH_WRITER_STATUS.lock().await;
            status.state = FlashUpdateState::WriteError;
            status.error_details = Some(String::from("Failed to commit update"));
        }
    }
}

#[task]
pub async fn flash_writer_task(flash: &'static MFlashType<'static>) {
    let receiver = FLASH_WRITE_REQUEST_CHANNEL.receiver();

    loop {
        // Wait for a block to write
        let received_command = receiver.receive().await;

        let state = { FLASH_WRITER_STATUS.lock().await.state };

        match state {
            FlashUpdateState::Idle | FlashUpdateState::Complete => {
                match received_command {
                    FlashWriteCommand::StartUpdate { total_size } => {
                        // If we receive a start update request, we can start the update
                        // and set the state to updating
                        debug!("Starting update with total size: {}", total_size);
                        let num_blocks = total_size.div_ceil(FLASH_WRITE_BLOCK_SIZE as u32) as u16;

                        start_update(flash, total_size, num_blocks).await;
                    }
                    FlashWriteCommand::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    _ => {
                        // Any other request results in an error
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::ProtocolError;
                        status.error_details = Some("Invalid request in idle state".into());
                    }
                }
            }
            FlashUpdateState::Updating => {
                match received_command {
                    FlashWriteCommand::StartUpdate { total_size } => {
                        // Restarting the update is always allowed
                        debug!("Restarting update with total size: {}", total_size);
                        let num_blocks = total_size.div_ceil(FLASH_WRITE_BLOCK_SIZE as u32) as u16;
                        start_update(flash, total_size, num_blocks).await;
                    }
                    FlashWriteCommand::WriteBlock { block_num, data } => {
                        let data_len = data.len();
                        let offset = (FLASH_WRITE_BLOCK_SIZE as u32) * block_num as u32;
                        debug!("Writing block {} at offset {}", block_num, offset);
                        let write_result = write_block(flash, offset, data).await;
                        match write_result {
                            Ok(_) => {
                                // Block written successfully
                                let mut status = FLASH_WRITER_STATUS.lock().await;
                                status.blocks_received += 1;
                                status.offset += data_len as u32;
                                if status.blocks_received >= status.total_num_blocks {
                                    // All blocks received, ready to commit
                                    status.state = FlashUpdateState::ReadyToCommit;
                                }
                            }
                            Err(e) => {
                                // Handle write error
                                let mut status = FLASH_WRITER_STATUS.lock().await;
                                status.state = FlashUpdateState::WriteError;
                                status.error_details = Some(e);
                            }
                        }
                    }
                    FlashWriteCommand::Abort => {
                        // Abort request is always accepted
                        debug!("Aborting update");
                        abort_update().await;
                    }
                    FlashWriteCommand::Commit => {
                        // Commit request is only accepted in ReadyToCommit state
                        debug!("Committing update");
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::ProtocolError;
                        status.error_details = Some("Invalid request in updating state".into());
                    }
                }
            }
            FlashUpdateState::ReadyToCommit => {
                match received_command {
                    FlashWriteCommand::StartUpdate { total_size } => {
                        // Restarting the update is always allowed
                        let num_blocks = total_size.div_ceil(FLASH_WRITE_BLOCK_SIZE as u32) as u16;
                        start_update(flash, total_size, num_blocks).await;
                    }
                    FlashWriteCommand::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    FlashWriteCommand::WriteBlock { .. } => {
                        // Write request is not allowed in ReadyToCommit state
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::ProtocolError;
                        status.error_details =
                            Some("Invalid request in ready to commit state".into());
                    }
                    FlashWriteCommand::Commit => {
                        // Commit request is only accepted in ReadyToCommit state
                        commit_update(flash).await;
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::Complete;
                    }
                }
            }
            FlashUpdateState::Preparing
            | FlashUpdateState::WriteError
            | FlashUpdateState::ProtocolError => {
                // If we are in a preparing or error state, we can only accept abort or start requests
                match received_command {
                    FlashWriteCommand::StartUpdate { total_size } => {
                        // Restarting the update is allowed even in error state
                        let num_blocks = total_size.div_ceil(FLASH_WRITE_BLOCK_SIZE as u32) as u16;
                        start_update(flash, total_size, num_blocks).await;
                    }
                    FlashWriteCommand::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    _ => {
                        // Any other request results in an error
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::ProtocolError;
                        status.error_details = Some("Invalid request in error state".into());
                    }
                }
            }
        }
    }
}
