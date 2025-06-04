use super::flash_writer::FLASH_WRITER_STATUS;
use crate::config::{
    FLASH_WRITE_BLOCK_SIZE, IIN_MAX_VALUE, MAX_TEMPERATURE_VALUE, MIN_TEMPERATURE_VALUE,
    VIN_MAX_VALUE, VSCAP_MAX_VALUE,
};
use crate::config_resources::I2CSecondaryResources;
use crate::tasks::config_manager::{
    get_vscap_power_off_threshold, get_vscap_power_on_threshold, set_vscap_power_off_threshold,
    set_vscap_power_on_threshold,
};
use crate::tasks::flash_writer::{
    FLASH_WRITE_REQUEST_CHANNEL, FlashUpdateState, FlashWriteCommand,
};
use crate::tasks::gpio_input::INPUTS;
use crate::tasks::host_watchdog::{
    HOST_WATCHDOG_EVENT_CHANNEL, HostWatchdogEvents, get_host_watchdog_timeout_ms,
    set_host_watchdog_timeout_ms,
};
use crate::tasks::led_blinker::{get_led_brightness, set_led_brightness};
use crate::tasks::state_machine::{
    OffState, STATE_MACHINE_EVENT_CHANNEL, SleepShutdownState, StateMachine, StateMachineEvents,
};
use alloc::vec::Vec;
use crc::{CRC_32_ISO_HDLC, Crc};
use defmt::{debug, error, info};
use embassy_executor::task;
use embassy_rp::peripherals::I2C1;
use embassy_rp::{bind_interrupts, i2c, i2c_slave};

// Following commands are supported by the I2C secondary interface:
// - Read 0x01: Query legacy hardware version
// - Read 0x02: Query legacy firmware version
// - Read 0x03: Query hardware version
// - Read 0x04: Query firmware version
// - Read 0x10: Query Raspi power state
// - Write 0x10 0x00: Set Raspi power off
// - Write 0x10 0x01: Set Raspi power on (who'd ever send that?)
// - Read 0x12: Query watchdog timeout
// - Write 0x12 [NN]: Set watchdog timeout to 0.1*NN seconds
// - Write 0x12 0x00: Disable watchdog
// - Read 0x13: Query power-on supercap threshold voltage
// - Write 0x13 [NN]: Set power-on supercap threshold voltage to 0.01*NN V
// - Read 0x14: Query power-off supercap threshold voltage
// - Write 0x14 [NN]: Set power-off supercap threshold voltage to 0.01*NN V
// - Read 0x15: Query state machine state
// - Read 0x16: Query watchdog elapsed (always returns 0x00)
// - Read 0x17: Query LED brightness setting
// - Write 0x17 [NN]: Set LED brightness to NN
// - Read 0x20: Query DC IN voltage
// - Read 0x21: Query supercap voltage
// - Read 0x22: Query DC IN current
// - Read 0x23: Query MCU temperature
// - Write 0x30: [ANY]: Initiate shutdown
// - Write 0x31: [ANY]: Initiate sleep shutdown
// - Write 0x40: [NNNN]: Start DFU (device firmware update), binary size is NNNN bytes
// - Read 0x41: Read DFU status
// - Read 0x42: Read Number of Block Written
// - Write 0x43: Upload a block of DFU data, serialized with postcard
// - Write 0x44: Commit the uploaded DFU data
// - Write 0x45: Abort the DFU process

// DFU protocol:
//
// Every DFU write command must be followed by a Read DFU Status command (0x41) to get
// the current status.
//
// An update always starts with a StartDFU command (0x40). Then,
// a sequence of UploadBlock commands (0x43) is sent, each containing a block of
// firmware data. The blocks are sent in order, starting from block 0. The
// block size is 4096, and the last block may be smaller
// than the others. If the status query indicates that the write queue is full,
// the sender should wait for a short time before retrying to send the next block.
// After all blocks are sent, a CommitDFU command (0x44) is sent to finalize the update.
// If the update needs to be aborted, an AbortDFU command (0x45) can be sent at any time.

const I2C_ADDR: u8 = 0x6d;

const LEGACY_FW_VERSION: u8 = 0xff;
const LEGACY_HW_VERSION: u8 = 0x00;

const FW_VERSION: [u8; 4] = [3, 0, 0, 0x01];
const HW_VERSION: [u8; 4] = [3, 0, 0, 0x02];

bind_interrupts!(struct Irqs {
    I2C1_IRQ => i2c::InterruptHandler<I2C1>;
});

#[repr(u8)]
pub enum DFUState {
    Idle = 0,
    Preparing = 1,
    Updating = 2,
    QueueFull = 3,
    ReadyToCommit = 4,
    CRCError = 5,
    DataLengthError = 6,
    WriteError = 7,
    ProtocolError = 8,
}

async fn get_dfu_state(crc_error: bool, data_length_error: bool) -> DFUState {
    let flash_writer_status = FLASH_WRITER_STATUS.lock().await;
    let flash_writer_state = flash_writer_status.state;
    let ready_for_more = !FLASH_WRITE_REQUEST_CHANNEL.is_full();

    match (
        flash_writer_state,
        crc_error,
        data_length_error,
        ready_for_more,
    ) {
        (_, true, _, _) => DFUState::CRCError,
        (_, _, true, _) => DFUState::DataLengthError,
        (FlashUpdateState::ProtocolError, _, _, _) => DFUState::ProtocolError,
        (FlashUpdateState::Idle | FlashUpdateState::Complete, _, _, _) => DFUState::Idle,
        (FlashUpdateState::Preparing, _, _, _) => DFUState::Preparing,
        (FlashUpdateState::WriteError, _, _, _) => DFUState::WriteError,
        (FlashUpdateState::Updating, _, _, false) => DFUState::QueueFull,
        (FlashUpdateState::Updating, _, _, _) => DFUState::Updating,
        (FlashUpdateState::ReadyToCommit, _, _, _) => DFUState::ReadyToCommit,
    }
}

async fn respond(device: &mut i2c_slave::I2cSlave<'_, I2C1>, data: &[u8]) {
    if let Err(e) = device.respond_and_fill(data, 0x00).await {
        error!("error while responding {}", e)
    }
}

#[task]
pub async fn i2c_secondary_task(r: I2CSecondaryResources) {
    info!("Starting I2C secondary task");
    let mut config = i2c_slave::Config::default();
    config.addr = I2C_ADDR as u16;
    let mut device = i2c_slave::I2cSlave::new(r.i2c, r.scl, r.sda, Irqs, config);
    let mut dfu_crc_error: bool = false;
    let mut data_length_error: bool = false;

    let state = 0;

    info!("I2C secondary task initialized");

    loop {
        let mut buf = [0u8; FLASH_WRITE_BLOCK_SIZE + 10];
        match device.listen(&mut buf).await {
            Ok(i2c_slave::Command::GeneralCall(len)) => {
                error!("General call write received: {}", buf[..len]);
            }
            Ok(i2c_slave::Command::Read) => loop {
                match device.respond_to_read(&[state]).await {
                    Ok(x) => match x {
                        i2c_slave::ReadStatus::Done => break,
                        i2c_slave::ReadStatus::NeedMoreBytes => (),
                        i2c_slave::ReadStatus::LeftoverBytes(x) => {
                            info!("Left over bytes: {:?}", x);
                            break;
                        }
                    },
                    Err(e) => {
                        error!("Error responding to read: {:?}", e);
                    }
                }
            },
            Ok(i2c_slave::Command::Write(len)) => {
                if len < 2 {
                    error!("Write command too short");
                    continue;
                }

                match buf[0] {
                    // Set Raspi power off/on
                    0x10 => {
                        match buf[1] {
                            0x00 => {
                                let inputs = INPUTS.lock().await;
                                if inputs.pg_5v {
                                    // Power off the Raspi
                                    info!("Powering off the Raspi");
                                    let new_state = StateMachine::Off(OffState::new());
                                    STATE_MACHINE_EVENT_CHANNEL
                                        .send(StateMachineEvents::SetState(new_state))
                                        .await;
                                } else {
                                    error!("Raspi power is already off");
                                }
                            }
                            _ => {
                                error!("Invalid power state: {}", buf[1]);
                            }
                        }
                    }
                    // Set watchdog timeout
                    0x12 => {
                        if len != 3 {
                            // Need exactly 3 bytes for the timeout value
                            error!("Invalid watchdog timeout command length");
                        } else {
                            let timeout = u16::from_be_bytes([buf[1], buf[2]]);
                            info!("Setting watchdog timeout to {} ms", timeout);
                            set_host_watchdog_timeout_ms(timeout).await;
                        }
                    }
                    // Set supercap power-on threshold voltage
                    0x13 => {
                        if len != 3 {
                            // Need exactly 3 bytes for the threshold value
                            error!("Invalid power-on threshold command length");
                        } else {
                            let cthreshold = u16::from_be_bytes([buf[1], buf[2]]);
                            let threshold: f32 = cthreshold as f32 / 100.0; // Convert to millivolts (0.01*NN V)
                            info!("Setting power-on threshold to {} V", threshold);
                            set_vscap_power_on_threshold(threshold).await;
                        }
                    }
                    // Set supercap power-off threshold voltage
                    0x14 => {
                        if len != 3 {
                            // Need exactly 3 bytes for the threshold value
                            error!("Invalid power-off threshold command length");
                        } else {
                            let cthreshold = u16::from_be_bytes([buf[1], buf[2]]);
                            let threshold: f32 = cthreshold as f32 / 100.0; // Convert to millivolts (0.01*NN V)
                            info!("Setting power-off threshold to {} V", threshold);
                            set_vscap_power_off_threshold(threshold).await;
                        }
                    }
                    // Set LED brightness
                    0x17 => {
                        let brightness = buf[1];
                        info!("Setting LED brightness to {}", brightness);
                        set_led_brightness(brightness).await;
                    }
                    // Initiate shutdown
                    0x30 => {
                        info!("Initiating shutdown");
                        let new_state = StateMachine::Off(OffState::new());
                        STATE_MACHINE_EVENT_CHANNEL
                            .send(StateMachineEvents::SetState(new_state))
                            .await;
                    }
                    // Initiate sleep shutdown
                    0x31 => {
                        info!("Initiating sleep shutdown");
                        let new_state = StateMachine::SleepShutdown(SleepShutdownState::new());
                        STATE_MACHINE_EVENT_CHANNEL
                            .send(StateMachineEvents::SetState(new_state))
                            .await;
                    }
                    // Start DFU process
                    0x40 => {
                        // Message payload is an u32 with the size of the firmware binary
                        if len < 5 {
                            error!("Invalid DFU start command length");
                            data_length_error = true;
                            continue;
                        }
                        let size = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
                        info!("Starting DFU process");
                        dfu_crc_error = false;
                        data_length_error = false;
                        FLASH_WRITE_REQUEST_CHANNEL
                            .send(FlashWriteCommand::StartUpdate { total_size: size })
                            .await;
                    }
                    // Upload a block of DFU data
                    0x43 => {
                        if len < 10 {
                            error!("Invalid DFU upload block command length");
                            data_length_error = true;
                            continue;
                        }
                        if dfu_crc_error || data_length_error {
                            // If there was a CRC error or block length error, skip processing
                            error!("Skipping DFU block upload due to previous errors");
                            continue;
                        }
                        data_length_error = false;
                        let crc_checksum = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]);
                        let payload = &buf[5..len];
                        let block_num = u16::from_be_bytes([buf[5], buf[6]]);
                        let block_length = u16::from_be_bytes([buf[7], buf[8]]);
                        let dfu_data = &buf[9..len].to_vec();

                        // Verify the CRC32 checksum
                        let crc = Crc::<u32>::new(&CRC_32_ISO_HDLC);
                        let calculated_crc = crc.checksum(payload);

                        if calculated_crc != crc_checksum {
                            error!(
                                "DFU block CRC mismatch: expected 0x{:08x}, got 0x{:08x}",
                                crc_checksum, calculated_crc
                            );
                            dfu_crc_error = true;
                            continue;
                        }
                        dfu_crc_error = false;

                        // Validate the block length
                        if block_length as usize != dfu_data.len() {
                            error!(
                                "DFU block length mismatch: expected {}, got {}",
                                block_length,
                                dfu_data.len()
                            );
                            data_length_error = true;
                            continue;
                        }
                        debug!(
                            "Uploading DFU block: block_num = {}, data length = {}",
                            block_num,
                            dfu_data.len()
                        );

                        FLASH_WRITE_REQUEST_CHANNEL
                            .send(FlashWriteCommand::WriteBlock {
                                block_num,
                                data: dfu_data.clone(),
                            })
                            .await;
                    }
                    // Commit the DFU update
                    0x44 => {
                        if dfu_crc_error || data_length_error {
                            // If there was a CRC error or block length error, skip processing
                            error!("Skipping DFU commit due to previous errors");
                            continue;
                        }
                        info!("Committing DFU update");
                        FLASH_WRITE_REQUEST_CHANNEL
                            .send(FlashWriteCommand::Commit)
                            .await;
                    }
                    // Abort the DFU update
                    0x45 => {
                        info!("Aborting DFU update");
                        FLASH_WRITE_REQUEST_CHANNEL
                            .send(FlashWriteCommand::Abort)
                            .await;
                    }
                    x => error!("Invalid Write command: {:02x}", x),
                }
            }
            Ok(i2c_slave::Command::WriteRead(len)) => {
                let inputs = INPUTS.lock().await;
                match buf[0] {
                    // Query legacy hardware version
                    0x01 => respond(&mut device, &[LEGACY_HW_VERSION]).await,
                    // Query legacy firmware version
                    0x02 => respond(&mut device, &[LEGACY_FW_VERSION]).await,
                    // Query hardware version
                    0x03 => {
                        debug!("Querying hardware version");
                        respond(&mut device, &HW_VERSION).await
                    }
                    // Query firmware version
                    0x04 => respond(&mut device, &FW_VERSION).await,
                    // Query Raspi power state
                    0x10 => respond(&mut device, &[inputs.pg_5v as u8]).await,
                    // Query watchdog timeout
                    0x12 => {
                        let timeout = get_host_watchdog_timeout_ms().await;
                        let timeout_bytes = timeout.to_be_bytes();
                        respond(&mut device, &timeout_bytes).await
                    }
                    // Query power-on threshold voltage
                    0x13 => {
                        let threshold = get_vscap_power_on_threshold().await;
                        let threshold_centi = (100.0 * threshold) as u16;
                        let threshold_bytes = threshold_centi.to_be_bytes();
                        respond(&mut device, &threshold_bytes).await
                    }
                    // Query power-off threshold voltage
                    0x14 => {
                        let threshold = get_vscap_power_off_threshold().await;
                        debug!("power off threshold: {}", threshold);
                        let threshold_centi = (100.0 * threshold) as u16;
                        debug!("power off threshold centi: {}", threshold_centi);
                        let msb_bytes = threshold_centi.to_be_bytes();
                        respond(&mut device, &msb_bytes).await
                    }
                    // Query state machine state
                    0x15 => {
                        // TODO: Implement state machine state query
                        // For now, return a placeholder value
                        let state_value: u8 = 0x01; // Placeholder
                        respond(&mut device, &[state_value]).await
                    }
                    // Query watchdog elapsed time (always returns 0x00)
                    0x16 => respond(&mut device, &[0]).await,
                    // Query LED brightness setting
                    0x17 => {
                        let brightness = get_led_brightness().await;
                        respond(&mut device, &[brightness]).await
                    }
                    // Query DC IN voltage
                    0x20 => {
                        let voltage = inputs.vin;
                        let voltage_bytes =
                            ((65535.0 * voltage / VIN_MAX_VALUE) as u16).to_be_bytes();
                        respond(&mut device, &voltage_bytes).await
                    }
                    // Query supercap voltage
                    0x21 => {
                        let voltage = inputs.vscap;
                        let voltage_bytes =
                            ((65536.0 * voltage / VSCAP_MAX_VALUE) as u16).to_be_bytes();
                        respond(&mut device, &voltage_bytes).await
                    }
                    // Query DC IN current
                    0x22 => {
                        let current = inputs.iin;
                        let current_bytes =
                            ((65535.0 * current / IIN_MAX_VALUE) as u16).to_be_bytes();
                        respond(&mut device, &current_bytes).await
                    }
                    // Query MCU temperature
                    0x23 => {
                        let temp = inputs.mcu_temp;
                        let temp_bytes = ((65535.0 * (temp - MIN_TEMPERATURE_VALUE)
                            / (MAX_TEMPERATURE_VALUE - MIN_TEMPERATURE_VALUE))
                            as u16)
                            .to_be_bytes();
                        respond(&mut device, &temp_bytes).await
                    }
                    // Read DFU status
                    0x41 => {
                        let dfu_state = get_dfu_state(dfu_crc_error, data_length_error).await;
                        respond(&mut device, &[dfu_state as u8]).await
                    }
                    // Read number of blocks written
                    0x42 => {
                        let blocks_written = FLASH_WRITER_STATUS.lock().await.blocks_received;
                        let blocks_written_bytes = blocks_written.to_be_bytes();
                        respond(&mut device, &blocks_written_bytes).await
                    }
                    // Multi-byte ping
                    0x50 => {
                        debug!("Received ping command: {:02x}", buf[1..len]);
                        let response = &buf[1..len];
                        // Reverse the response bytes
                        let response: Vec<u8> = response.iter().rev().cloned().collect();
                        respond(&mut device, &response).await;
                        debug!("Responded to ping command");
                    }
                    x => error!("Invalid Write Read command: 0x{:02x}", x),
                }
            }
            Err(e) => error!("{}", e),
        }
        // Update watchdog on any I2C activity
        HOST_WATCHDOG_EVENT_CHANNEL
            .send(HostWatchdogEvents::Ping)
            .await;
    }
}
