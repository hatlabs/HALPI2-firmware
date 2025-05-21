use crate::config::{
    IIN_MAX_VALUE, MAX_TEMPERATURE_VALUE, MIN_TEMPERATURE_VALUE,
    VIN_MAX_VALUE, VSCAP_MAX_VALUE,
};
use crate::config_resources::I2CSecondaryResources;
use crate::flash_config::{
    get_vscap_power_off_threshold, get_vscap_power_on_threshold, set_vscap_power_off_threshold,
    set_vscap_power_on_threshold,
};
use crate::tasks::flash_writer::{FlashWriteRequest, FLASH_WRITE_REQUEST_CHANNEL};
use crate::tasks::gpio_input::INPUTS;
use crate::tasks::host_watchdog::{
    HOST_WATCHDOG_EVENT_CHANNEL, HostWatchdogEvents, get_host_watchdog_timeout_ms,
    set_host_watchdog_timeout_ms,
};
use crate::tasks::led_blinker::{get_led_brightness, set_led_brightness};
use crate::tasks::state_machine::{
    OffState, STATE_MACHINE_EVENT_CHANNEL, SleepShutdownState, StateMachine, StateMachineEvents,
};
use alloc::string::String;
use alloc::vec::Vec;
use crc::{Crc, CRC_32_ISCSI};
use defmt::{debug, error, info};
use embassy_executor::task;
use embassy_rp::peripherals::I2C0;
use embassy_rp::{bind_interrupts, i2c, i2c_slave};
use super::flash_writer::FLASH_WRITER_STATUS;

use shared_types::{
    FlashUpdateCommand, FlashUpdateResponse, FlashUpdateState,
};

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
// - Write 0x40: [NN]: Send flash update command

const I2C_ADDR: u8 = 0x6d;

const LEGACY_FW_VERSION: u8 = 0xff;
const LEGACY_HW_VERSION: u8 = 0x00;

const FW_VERSION: [u8; 4] = [3, 0, 0, 0x01];
const HW_VERSION: [u8; 4] = [3, 0, 0, 0x02];

bind_interrupts!(struct Irqs {
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

async fn render_status_response() -> Vec<u8> {
    let flash_writer_status = FLASH_WRITER_STATUS.lock().await;
    let state = flash_writer_status.state;
    let blocks_written = flash_writer_status.blocks_received;
    let ready_for_more = !FLASH_WRITE_REQUEST_CHANNEL.is_full();
    let error_details = flash_writer_status.error_details.clone();

    let response = FlashUpdateResponse::Status {
        state,
        blocks_written,
        ready_for_more,
        error_details,
    };

    let crc = Crc::<u32>::new(&CRC_32_ISCSI);
    let digest = crc.digest();
    postcard::to_allocvec_crc32(&response, digest).unwrap()
}

fn render_ack_response(success: bool, error_code: Option<u8>) -> Vec<u8> {
    let response = FlashUpdateResponse::Ack {
        success,
        error_code,
    };

    let crc = Crc::<u32>::new(&CRC_32_ISCSI);
    let digest = crc.digest();
    postcard::to_allocvec_crc32(&response, digest).unwrap()
}

async fn handle_flash_update_command(buffer: &[u8]) -> Result<Vec<u8>, String> {
    let crc = Crc::<u32>::new(&CRC_32_ISCSI);
    let digest = crc.digest();
    let result = postcard::from_bytes_crc32(buffer, digest);
    let command: FlashUpdateCommand = match result {
        Ok(command) => command,
        Err(e) => {
            error!("Failed to parse command: {:?}", defmt::Debug2Format(&e));
            return Err("Failed to parse command".into());
        }
    };

    match command {
        FlashUpdateCommand::StartUpdate { num_blocks, total_size, expected_crc32 } => {
            info!("Starting firmware update: total size = {}, expected CRC32 = {}", total_size, expected_crc32);
            // FIXME: Handle expected CRC32 check
            FLASH_WRITE_REQUEST_CHANNEL
                .send(FlashWriteRequest::StartUpdate {
                    num_blocks,
                    total_size,
                })
                .await;
            // Send acknowledgment
            let digest = crc.digest();
            let response = FlashUpdateResponse::Ack {
                success: true,
                error_code: None,
            };
            let response_bytes: Vec<u8> = postcard::to_allocvec_crc32(&response, digest).unwrap();
            Ok(response_bytes)
        }
        FlashUpdateCommand::UploadBlock { block_num, data, block_crc } => {
            info!("Uploading block: block_num = {}, block CRC = {}", block_num, block_crc);
            FLASH_WRITE_REQUEST_CHANNEL
                .send(FlashWriteRequest::WriteBlock {
                    block_num,
                    data,
                })
                .await;
            // Send acknowledgment
            Ok(render_ack_response(true, None))
        }
        FlashUpdateCommand::GetStatus => {
            info!("Getting update status");
            let response = render_status_response().await;
            Ok(response)
        }
        FlashUpdateCommand::CommitUpdate => {
            info!("Committing firmware update");
            FLASH_WRITE_REQUEST_CHANNEL
                .send(FlashWriteRequest::Commit)
                .await;
            Ok(render_ack_response(true, None))
        }
        FlashUpdateCommand::AbortUpdate => {
            info!("Aborting firmware update");
            FLASH_WRITE_REQUEST_CHANNEL
                .send(FlashWriteRequest::Abort)
                .await;
            Ok(render_ack_response(true, None))
        }
    }
}

#[task]
pub async fn i2c_secondary_task(r: I2CSecondaryResources) {
    info!("Starting I2C secondary task");
    let mut config = i2c_slave::Config::default();
    config.addr = I2C_ADDR as u16;
    let mut device = i2c_slave::I2cSlave::new(r.i2c, r.scl, r.sda, Irqs, config);

    let state = 0;

    info!("I2C secondary task initialized");

    loop {
        // Handle I2C secondary (slave) communication
        let mut buf = [0u8; 5000];
        match device.listen(&mut buf).await {
            Ok(i2c_slave::Command::GeneralCall(len)) => {
                // Handle general call write
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
                info!("Device received write: {}", buf[..len]);
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
                            0x01 => {
                                info!("Powering on the Raspi (not implemented)");
                                // Note: The comment suggests this is not needed/expected to be implemented
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
                    x => error!("Invalid Write command: 0x{:02x}", x),
                }
            }
            Ok(i2c_slave::Command::WriteRead(len)) => {
                info!("device received write read: {:x}", buf[..len]);
                match buf[0] {
                    // Query legacy hardware version
                    0x01 => match device.respond_and_fill(&[LEGACY_HW_VERSION], 0x00).await {
                        Ok(read_status) => info!("response read status {}", read_status),
                        Err(e) => error!("error while responding {}", e),
                    },
                    // Query legacy firmware version
                    0x02 => match device.respond_and_fill(&[LEGACY_FW_VERSION], 0x00).await {
                        Ok(read_status) => info!("response read status {}", read_status),
                        Err(e) => error!("error while responding {}", e),
                    },
                    // Query hardware version
                    0x03 => match device.respond_and_fill(&HW_VERSION, 0x00).await {
                        Ok(read_status) => info!("response read status {}", read_status),
                        Err(e) => error!("error while responding {}", e),
                    },
                    // Query firmware version
                    0x04 => match device.respond_and_fill(&FW_VERSION, 0x00).await {
                        Ok(read_status) => info!("response read status {}", read_status),
                        Err(e) => error!("error while responding {}", e),
                    },
                    // Query Raspi power state
                    0x10 => {
                        let inputs = INPUTS.lock().await;
                        match device.respond_and_fill(&[inputs.pg_5v as u8], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query watchdog timeout
                    0x12 => {
                        let timeout = get_host_watchdog_timeout_ms().await;
                        let timeout_bytes = timeout.to_be_bytes();
                        match device.respond_and_fill(&timeout_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query power-on threshold voltage
                    0x13 => {
                        let threshold = get_vscap_power_on_threshold().await;
                        let threshold_centi = (100.0 * threshold) as u16; // Convert to centivolt
                        let threshold_bytes = threshold_centi.to_be_bytes();
                        match device.respond_and_fill(&threshold_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query power-off threshold voltage
                    0x14 => {
                        let threshold = get_vscap_power_off_threshold().await;
                        debug!("power off threshold: {}", threshold);
                        let threshold_centi = (100.0 * threshold) as u16; // Convert to centivolt
                        debug!("power off threshold centi: {}", threshold_centi);
                        let msb_bytes = threshold_centi.to_be_bytes();
                        match device.respond_and_fill(&msb_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query state machine state
                    0x15 => {
                        // TODO: Implement state machine state query
                        // For now, return a placeholder value
                        let state_value: u8 = 0x01; // Placeholder
                        match device.respond_and_fill(&[state_value], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query watchdog elapsed time (always returns 0x00)
                    0x16 => match device.respond_and_fill(&[0], 0x00).await {
                        Ok(read_status) => info!("response read status {}", read_status),
                        Err(e) => error!("error while responding {}", e),
                    },
                    // Query LED brightness setting
                    0x17 => {
                        let brightness = get_led_brightness().await;
                        match device.respond_and_fill(&[brightness], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query DC IN voltage
                    0x20 => {
                        let voltage = INPUTS.lock().await.vin;
                        let voltage_bytes =
                            ((65535.0 * voltage / VIN_MAX_VALUE) as u16).to_be_bytes();
                        match device.respond_and_fill(&voltage_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query supercap voltage
                    0x21 => {
                        let voltage = INPUTS.lock().await.vscap;
                        let voltage_bytes =
                            ((65536.0 * voltage / VSCAP_MAX_VALUE) as u16).to_be_bytes();
                        match device.respond_and_fill(&voltage_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query DC IN current
                    0x22 => {
                        let current = INPUTS.lock().await.iin;
                        let current_bytes =
                            ((65535.0 * current / IIN_MAX_VALUE) as u16).to_be_bytes();
                        match device.respond_and_fill(&current_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query MCU temperature
                    0x23 => {
                        let temp = INPUTS.lock().await.mcu_temp;
                        // Scale the temperatures between MIN_TEMPERATURE_VALUE and
                        // MAX_TEMPERATURE_VALUE to 0..65535
                        let temp_bytes = ((65535.0 * (temp - MIN_TEMPERATURE_VALUE)
                            / (MAX_TEMPERATURE_VALUE - MIN_TEMPERATURE_VALUE))
                            as u16)
                            .to_be_bytes();

                        match device.respond_and_fill(&temp_bytes, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Flash update command
                    0x40 => {
                        let result = handle_flash_update_command(&buf[1..len]).await;
                        let response = match result {
                            Ok(response) => response,
                            Err(_) => render_ack_response(false, Some(1)),
                        };
                        match device.respond_and_fill(&response, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }

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
