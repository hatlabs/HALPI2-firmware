use crate::config_resources::I2CSecondaryResources;
use crate::tasks::state_machine::{StateMachine, StateMachineEvents, WatchdogRebootState, STATE_MACHINE_EVENT_CHANNEL};
use defmt::{error, info, warn};
use embassy_executor::task;
use embassy_rp::peripherals::{I2C0, I2C1};
use embassy_rp::{bind_interrupts, i2c, i2c_slave};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant, Ticker, Timer};
use embedded_hal_async::i2c::I2c;
use crate::tasks::gpio_input::INPUTS;

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
// - Read 0x13: Query power-on threshold voltage
// - Write 0x13 [NN]: Set power-on threshold voltage to 0.01*NN V
// - Read 0x14: Query power-off threshold voltage
// - Write 0x14 [NN]: Set power-off threshold voltage to 0.01*NN V
// - Read 0x15: Query state machine state
// - Read 0x16: Query watchdog elapsed
// - Read 0x17: Query LED brightness setting
// - Write 0x17 [NN]: Set LED brightness to NN
// - Read 0x20: Query DC IN voltage
// - Read 0x21: Query supercap voltage
// - Read 0x22: Query DC IN current
// - Read 0x23: Query MCU temperature
// - Write 0x30: [ANY]: Initiate shutdown
// - Write 0x31: [ANY]: Initiate sleep shutdown

const I2C_ADDR: u8 = 0x6d;

const LEGACY_FW_VERSION: u8 = 0xff;
const LEGACY_HW_VERSION: u8 = 0x00;

const FW_VERSION: [u8; 4] = [3, 0, 0, 0x01];
const HW_VERSION: [u8; 4] = [3, 0, 0, 0x02];

bind_interrupts!(struct Irqs {
    I2C0_IRQ => i2c::InterruptHandler<I2C0>;
});

pub enum HostWatchdogEvents {
    Ping,
    SetTimeoutMs(u32),
}

type HostWatchdogChannelType =
    embassy_sync::channel::Channel<CriticalSectionRawMutex, HostWatchdogEvents, 8>;
static HOST_WATCHDOG_EVENT_CHANNEL: HostWatchdogChannelType =
    embassy_sync::channel::Channel::new();

// Run a watchdog task that will reset the system if it doesn't receive a ping
// within a certain time frame. Any I2C command will reset the watchdog.
#[task]
pub async fn host_watchdog_task() {
    let mut last_ping = Instant::now();
    let mut timeout_ms = 10000;

    let mut ticker = Ticker::every(Duration::from_millis(100));
    let receiver = HOST_WATCHDOG_EVENT_CHANNEL.receiver();
    loop {
        ticker.next().await;
        if !receiver.is_empty() {
            let event = receiver.receive().await;
            match event {
                HostWatchdogEvents::Ping => {
                    last_ping = Instant::now();
                }
                HostWatchdogEvents::SetTimeoutMs(timeout) => {
                    timeout_ms = timeout;
                }
            }
        }
        // Check if the watchdog has been pinged
        let now = Instant::now();
        if now.duration_since(last_ping) > Duration::from_millis(timeout_ms as u64) {
            // Reset the system
            warn!("Watchdog timeout");
            let new_state = StateMachine::WatchdogReboot(WatchdogRebootState::new());
            STATE_MACHINE_EVENT_CHANNEL.send(StateMachineEvents::SetState(new_state)).await;
            last_ping = now;
        }
    }
}

#[task]
pub async fn i2c_secondary_task(r: I2CSecondaryResources) {
    let mut config = i2c_slave::Config::default();
    config.addr = I2C_ADDR as u16;
    let mut device = i2c_slave::I2cSlave::new(r.i2c, r.scl, r.sda, Irqs, config);

    let state = 0;

    loop {
        // Handle I2C secondary (slave) communication
        let mut buf = [0u8; 128];
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
            Ok(i2c_slave::Command::Write(len)) => info!("Device received write: {}", buf[..len]),
            Ok(i2c_slave::Command::WriteRead(len)) => {
                info!("device received write read: {:x}", buf[..len]);
                match buf[0] {
                    // Query legacy hardware version
                    0x01 => {
                        match device.respond_and_fill(&[LEGACY_HW_VERSION], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query legacy firmware version
                    0x02 => {
                        match device.respond_and_fill(&[LEGACY_FW_VERSION], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query hardware version
                    0x03 => {
                        match device.respond_and_fill(&HW_VERSION, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query firmware version
                    0x04 => {
                        match device.respond_and_fill(&FW_VERSION, 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }
                    // Query Raspi power state
                    0x10 => {
                        let inputs = INPUTS.lock().await;
                        match device.respond_and_fill(&[inputs.pg_5v as u8], 0x00).await {
                            Ok(read_status) => info!("response read status {}", read_status),
                            Err(e) => error!("error while responding {}", e),
                        }
                    }

                    x => error!("Invalid Write Read {:x}", x),
                }
            }

            Err(e) => error!("{}", e),
        }
        HOST_WATCHDOG_EVENT_CHANNEL
            .send(HostWatchdogEvents::Ping)
            .await;
    }
}
