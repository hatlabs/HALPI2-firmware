use defmt::{warn, Format};
use embassy_executor::task;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Instant, Ticker};

use crate::{config::HOST_WATCHDOG_DEFAULT_TIMEOUT_MS, tasks::state_machine::{StateMachine, StateMachineEvents, WatchdogRebootState, STATE_MACHINE_EVENT_CHANNEL}};

#[derive(Clone, Format)]
struct HostWatchdogConfig {
    timeout_ms: u16,
    last_ping: Instant,
    enabled: bool,
}

impl Default for HostWatchdogConfig {
    fn default() -> Self {
        Self {
            timeout_ms: HOST_WATCHDOG_DEFAULT_TIMEOUT_MS,
            last_ping: Instant::now(),
            enabled: false,
        }
    }
}

impl HostWatchdogConfig {
    const fn new(timeout_ms: u16) -> Self {
        Self {
            timeout_ms,
            last_ping: Instant::MIN,
            enabled: false,
        }
    }
}

static HOST_WATCHDOG_CONFIG: Mutex<CriticalSectionRawMutex, HostWatchdogConfig> =
    Mutex::new(HostWatchdogConfig::new(HOST_WATCHDOG_DEFAULT_TIMEOUT_MS));

pub async fn get_host_watchdog_timeout_ms() -> u16 {
    let config = HOST_WATCHDOG_CONFIG.lock().await;
    config.timeout_ms
}

pub async fn set_host_watchdog_timeout_ms(timeout_ms: u16) {
    let mut config = HOST_WATCHDOG_CONFIG.lock().await;
    config.timeout_ms = timeout_ms;
    HOST_WATCHDOG_EVENT_CHANNEL
        .send(HostWatchdogEvents::SetTimeoutMs(timeout_ms))
        .await;
}

pub enum HostWatchdogEvents {
    Ping,
    SetTimeoutMs(u16),
    EnableWatchdog(bool),
}

type HostWatchdogChannelType =
    embassy_sync::channel::Channel<CriticalSectionRawMutex, HostWatchdogEvents, 8>;
pub static HOST_WATCHDOG_EVENT_CHANNEL: HostWatchdogChannelType = embassy_sync::channel::Channel::new();

// Run a watchdog task that will reset the system if it doesn't receive a ping
// within a certain time frame. Any I2C command will reset the watchdog.
#[task]
pub async fn host_watchdog_task() {
    let mut last_ping = Instant::now();
    let mut timeout_ms = get_host_watchdog_timeout_ms().await;

    let mut ticker = Ticker::every(Duration::from_millis(100));
    let receiver = HOST_WATCHDOG_EVENT_CHANNEL.receiver();
    loop {
        ticker.next().await;
        if !receiver.is_empty() {
            let event = receiver.receive().await;
            match event {
                HostWatchdogEvents::Ping => {
                    last_ping = Instant::now();
                    // Update the stored last ping time
                    let mut config = HOST_WATCHDOG_CONFIG.lock().await;
                    config.last_ping = last_ping;
                }
                HostWatchdogEvents::SetTimeoutMs(timeout) => {
                    timeout_ms = timeout;
                }
                HostWatchdogEvents::EnableWatchdog(enabled) => {
                    let mut config = HOST_WATCHDOG_CONFIG.lock().await;
                    config.enabled = enabled;
                    if enabled {
                        last_ping = Instant::now();
                        // Update the stored last ping time
                        config.last_ping = last_ping;
                    }
                }
            }
        }
        // Check if the watchdog is enabled
        let config = HOST_WATCHDOG_CONFIG.lock().await;
        if !config.enabled {
            continue;
        }

        // Check if the watchdog has been pinged
        let now = Instant::now();
        if now.duration_since(last_ping) > Duration::from_millis(timeout_ms as u64)
            && timeout_ms > 0
        {
            // Reset the system
            warn!("Watchdog timeout");
            let new_state = StateMachine::WatchdogReboot(WatchdogRebootState::new());
            STATE_MACHINE_EVENT_CHANNEL
                .send(StateMachineEvents::SetState(new_state))
                .await;
            last_ping = now;
            // Update the stored last ping time
            let mut config = HOST_WATCHDOG_CONFIG.lock().await;
            config.last_ping = last_ping;
        }
    }
}
