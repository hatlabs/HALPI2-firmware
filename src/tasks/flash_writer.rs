// Flash updater task for handling firmware updates via I2C

use alloc::{string::String, vec::Vec};

use embassy_boot::{AlignedBuffer, FirmwareUpdater, FirmwareUpdaterConfig};
use embassy_executor::task;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel, mutex::Mutex};

// Import the NorFlash trait for async write support
use embedded_storage_async::nor_flash::NorFlash;

use crate::OM_FLASH;

use super::i2c_secondary::FlashUpdateState;

// Message passed from I2C task to Flash writer task
pub enum FlashWriteRequest {
    StartUpdate { total_size: u32, num_blocks: u32 },
    WriteBlock { block_num: u32, data: Vec<u8> },
    Commit,
    Abort,
}

// Status shared between tasks
pub struct WriterStatus {
    pub state: FlashUpdateState,
    pub blocks_received: u32,
    pub offset: u32,
    pub total_size: u32,
    pub total_num_blocks: u32,
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
    channel::Channel<CriticalSectionRawMutex, FlashWriteRequest, 2>;
pub static FLASH_WRITE_REQUEST_CHANNEL: FlashWriteRequestChannelType = channel::Channel::new();

// Shared status between tasks
pub static FLASH_WRITER_STATUS: Mutex<CriticalSectionRawMutex, WriterStatus> =
    Mutex::new(WriterStatus::new());

async fn prepare_update() -> Result<(), String> {
    let flash = OM_FLASH.get().await;
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut aligned = AlignedBuffer([0; 1]);
    let mut firmware_updater = FirmwareUpdater::new(config, &mut aligned.0);

    match firmware_updater.prepare_update().await {
        Ok(_) => Ok(()),
        Err(e) => {
            defmt::warn!("E: {:?}", defmt::Debug2Format(&e));
            Err("Failed to prepare update".into())
        }
    }
}

async fn write_block(offset: u32, data: Vec<u8>) -> Result<(), String> {
    // Get the flash partition (implement this according to your OM_FLASH type)
    let flash = OM_FLASH.get().await;
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut writer = config.dfu;

    let mut buf: AlignedBuffer<4096> = AlignedBuffer([0; 4096]);
    let len_data = data.len();

    defmt::debug!("writer created, writing data");

    buf.0[..len_data].copy_from_slice(&data);
    // Write the block to flash
    let write_result = writer.write(offset, &buf.0[..]).await;

    defmt::debug!("write result: {:?}", defmt::Debug2Format(&write_result));

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

async fn start_update(total_size: u32, num_blocks: u32) {
    match prepare_update().await {
        Ok(_) => {
            defmt::debug!("update prepared");
            let mut status = FLASH_WRITER_STATUS.lock().await;
            status.state = FlashUpdateState::Updating;
            status.total_size = total_size;
            status.total_num_blocks = num_blocks;
            status.blocks_received = 0;
            status.offset = 0;
        }
        Err(e) => {
            defmt::warn!("E: {:?}", defmt::Debug2Format(&e));
            let mut status = FLASH_WRITER_STATUS.lock().await;
            status.state = FlashUpdateState::WriteError;
            status.error_details = Some(e);
        }
    }
}

async fn abort_update() {
    let mut status = FLASH_WRITER_STATUS.lock().await;
    status.state = FlashUpdateState::Idle;
    status.blocks_received = 0;
    status.offset = 0;
    status.total_size = 0;
    status.total_num_blocks = 0;
    status.error_details = None;
}

async fn commit_update() {
    let flash = OM_FLASH.get().await;
    let config = FirmwareUpdaterConfig::from_linkerfile(flash, flash);
    let mut aligned = AlignedBuffer([0; 1]);
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
pub async fn flash_writer_task() {
    let receiver = FLASH_WRITE_REQUEST_CHANNEL.receiver();

    loop {
        // Wait for a block to write
        let write_req = receiver.receive().await;

        let state = FLASH_WRITER_STATUS.lock().await.state;

        match state {
            FlashUpdateState::Idle | FlashUpdateState::Complete => {
                match write_req {
                    FlashWriteRequest::StartUpdate {
                        total_size,
                        num_blocks,
                    } => {
                        // If we receive a start update request, we can start the update
                        // and set the state to updating
                        start_update(total_size, num_blocks).await;
                    }
                    FlashWriteRequest::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    _ => {
                        // Any other request results in an error
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::DataError;
                        status.error_details = Some("Invalid request in idle state".into());
                    }
                }
            }
            FlashUpdateState::Updating => {
                match write_req {
                    FlashWriteRequest::StartUpdate {
                        total_size,
                        num_blocks,
                    } => {
                        // Restarting the update is always allowed
                        start_update(total_size, num_blocks).await;
                    }
                    FlashWriteRequest::WriteBlock { block_num, data } => {
                        let data_len = data.len();
                        let write_result = write_block(block_num, data).await;
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
                    FlashWriteRequest::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    FlashWriteRequest::Commit => {
                        // Commit request is only accepted in ReadyToCommit state
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::DataError;
                        status.error_details = Some("Invalid request in updating state".into());
                    }
                }
            }
            FlashUpdateState::ReadyToCommit => {
                match write_req {
                    FlashWriteRequest::StartUpdate {
                        total_size,
                        num_blocks,
                    } => {
                        // Restarting the update is always allowed
                        start_update(total_size, num_blocks).await;
                    }
                    FlashWriteRequest::Abort => {
                        // Abort request is always accepted
                        abort_update().await;
                    }
                    FlashWriteRequest::WriteBlock { .. } => {
                        // Write request is not allowed in ReadyToCommit state
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::DataError;
                        status.error_details =
                            Some("Invalid request in ready to commit state".into());
                    }
                    FlashWriteRequest::Commit => {
                        // Commit request is only accepted in ReadyToCommit state
                        commit_update().await;
                        let mut status = FLASH_WRITER_STATUS.lock().await;
                        status.state = FlashUpdateState::Complete;
                    }
                }
            }
            _ => {
                // Any other state is considered an error
                let mut status = FLASH_WRITER_STATUS.lock().await;
                status.state = FlashUpdateState::DataError;
                status.error_details = Some("Invalid state".into());
            }
        }
    }
}
