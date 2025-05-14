use alloc::string::{String, ToString};
use core::ops::Range;
use defmt::{debug, info};
use embassy_executor::task;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{SerializationError, fetch_item, remove_item, store_item};
use serde::{Deserialize, Serialize};

use crate::config::*;

// Define a comprehensive error type
#[derive(Debug)]
pub enum ConfigError {
    // Flash operation errors
    Flash(embassy_rp::flash::Error),
    // Other storage errors
    Storage,
}

impl From<embassy_rp::flash::Error> for ConfigError {
    fn from(error: embassy_rp::flash::Error) -> Self {
        ConfigError::Flash(error)
    }
}

impl From<sequential_storage::Error<embassy_rp::flash::Error>> for ConfigError {
    fn from(_: sequential_storage::Error<embassy_rp::flash::Error>) -> Self {
        ConfigError::Storage
    }
}

impl From<SerializationError> for ConfigError {
    fn from(_: SerializationError) -> Self {
        ConfigError::Storage
    }
}

#[derive(defmt::Format)]
pub enum ConfigManagerEvents {
    VscapPowerOnThreshold(f32),
    VinPowerThreshold(f32),
    ShutdownWaitDurationMs(u32),
    WatchdogTimeoutMs(u16),
    LedBrightness(u8),
}

pub type ConfigManagerChannelType =
    channel::Channel<CriticalSectionRawMutex, ConfigManagerEvents, 8>;
pub static CONFIG_MANAGER_EVENT_CHANNEL: ConfigManagerChannelType = channel::Channel::new();

// Configuration manager using sequential-storage
pub struct ConfigManager<'a> {
    flash: Flash<'a, FLASH, Async, FLASH_SIZE>,
    flash_range: Range<u32>,
    data_buffer: [u8; 128],
}

impl<'a> ConfigManager<'a> {
    fn new(flash: Flash<'a, FLASH, Async, FLASH_SIZE>, offset: u32, size: u32) -> Self {
        let flash_range = offset..(offset + size);
        let data_buffer = [0u8; 128];

        Self {
            flash,
            flash_range,
            data_buffer,
        }
    }

    /// Store a serializable value
    pub async fn set<T: Serialize>(&mut self, key: u16, value: &T) -> Result<(), ConfigError>
    where
        T: for<'de> Deserialize<'de> + Serialize + for<'b> sequential_storage::map::Value<'b>,
    {
        debug!("Storing item with key: {}", key);

        store_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
            value,
        )
        .await
        .map_err(ConfigError::from)
    }

    // Retrieve a deserialized value or None if not found
    // Modified to remove the lifetime dependency on self
    pub async fn get<T>(&mut self, key: u16) -> Result<Option<T>, ConfigError>
    where
        T: for<'de> Deserialize<'de> + Serialize + for<'b> sequential_storage::map::Value<'b>,
    {
        debug!("Fetching item with key: {}", key);

        fetch_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await
        .map_err(ConfigError::from)
    }

    // Check if a key exists
    pub async fn contains_key(&mut self, key: u16) -> Result<bool, ConfigError> {
        let result = fetch_item::<u16, Option<bool>, _>(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await
        .map_err(ConfigError::from)?;

        Ok(result.is_some())
    }

    // Remove a key
    pub async fn remove(&mut self, key: u16) -> Result<(), ConfigError> {
        remove_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await
        .map_err(ConfigError::from)
    }
}

pub static CONFIG_MANAGER: OnceLock<Mutex<CriticalSectionRawMutex, ConfigManager<'static>>> =
    OnceLock::new();

/// Runtime configuration values, read from the flash storage and stored here
/// to prevent multiple reads from the flash.
struct RuntimeConfig {
    pub vscap_power_on_threshold: f32,
    pub vscap_power_off_threshold: f32,
    pub vin_power_threshold: f32,
    pub shutdown_wait_duration_ms: u32,
    pub watchdog_timeout_ms: u16,
    pub led_brightness: u8,
}

impl RuntimeConfig {
    const fn new(
        vscap_power_on_threshold: f32,
        vscap_power_off_threshold: f32,
        vin_power_threshold: f32,
        shutdown_wait_duration_ms: u32,
        watchdog_timeout_ms: u16,
        led_brightness: u8,
    ) -> Self {
        RuntimeConfig {
            vscap_power_on_threshold,
            vscap_power_off_threshold,
            vin_power_threshold,
            shutdown_wait_duration_ms,
            watchdog_timeout_ms,
            led_brightness,
        }
    }
}

static RUNTIME_CONFIG: Mutex<CriticalSectionRawMutex, RuntimeConfig> =
    Mutex::new(RuntimeConfig::new(
        DEFAULT_VSCAP_POWER_ON_THRESHOLD,
        DEFAULT_VSCAP_POWER_OFF_THRESHOLD,
        DEFAULT_VIN_POWER_THRESHOLD,
        DEFAULT_SHUTDOWN_WAIT_DURATION_MS,
        HOST_WATCHDOG_DEFAULT_TIMEOUT_MS,
        DEFAULT_LED_BRIGHTNESS,
    ));

pub async fn get_vscap_power_on_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_on_threshold
}
pub async fn get_vscap_power_off_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_off_threshold
}
pub async fn get_vin_power_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vin_power_threshold
}
pub async fn get_shutdown_wait_duration_ms() -> u32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.shutdown_wait_duration_ms
}
pub async fn get_watchdog_timeout_ms() -> u16 {
    let config = RUNTIME_CONFIG.lock().await;
    config.watchdog_timeout_ms
}
pub async fn get_led_brightness() -> u8 {
    let config = RUNTIME_CONFIG.lock().await;
    config.led_brightness
}
pub async fn set_vscap_power_on_threshold(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_on_threshold = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VscapPowerOnThreshold(value))
        .await;
}
pub async fn set_vscap_power_off_threshold(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_off_threshold = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VscapPowerOnThreshold(value))
        .await;
}
pub async fn set_vin_power_threshold(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vin_power_threshold = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VinPowerThreshold(value))
        .await;
}
pub async fn set_shutdown_wait_duration_ms(value: u32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.shutdown_wait_duration_ms = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::ShutdownWaitDurationMs(value))
        .await;
}
pub async fn set_watchdog_timeout_ms(value: u16) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.watchdog_timeout_ms = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::WatchdogTimeoutMs(value))
        .await;
}
pub async fn set_led_brightness(value: u8) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.led_brightness = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::LedBrightness(value))
        .await;
}

pub async fn init_config_manager(
    flash: embassy_rp::flash::Flash<'static, FLASH, Async, FLASH_SIZE>,
) {
    let config_manager = ConfigManager::new(flash, FLASH_CONFIG_OFFSET, FLASH_CONFIG_SIZE);
    if CONFIG_MANAGER.init(Mutex::new(config_manager)).is_err() {
        // Handle the error appropriately, e.g., log it or panic
        panic!("Failed to initialize CONFIG_MANAGER");
    }
    info!("Config manager initialized");

    let mut config_manager = CONFIG_MANAGER.get().await.lock().await;

    let vscap_power_on_threshold = config_manager
        .get::<f32>(VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(DEFAULT_VSCAP_POWER_ON_THRESHOLD);
    let vscap_power_off_threshold = config_manager
        .get::<f32>(VSCAP_POWER_OFF_THRESHOLD_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(DEFAULT_VSCAP_POWER_OFF_THRESHOLD);
    let vin_power_threshold = config_manager
        .get::<f32>(VIN_POWER_THRESHOLD_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(DEFAULT_VIN_POWER_THRESHOLD);
    let shutdown_wait_duration_ms = config_manager
        .get::<u32>(SHUTDOWN_WAIT_DURATION_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(DEFAULT_SHUTDOWN_WAIT_DURATION_MS);
    let watchdog_timeout_ms = config_manager
        .get::<u16>(HOST_WATCHDOG_TIMEOUT_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(HOST_WATCHDOG_DEFAULT_TIMEOUT_MS);
    let led_brightness = config_manager
        .get::<u8>(LED_BRIGHTNESS_CONFIG_KEY)
        .await
        .unwrap_or(None)
        .unwrap_or(DEFAULT_LED_BRIGHTNESS);

    let mut runtime_config = RUNTIME_CONFIG.lock().await;
    *runtime_config = RuntimeConfig::new(
        vscap_power_on_threshold,
        vscap_power_off_threshold,
        vin_power_threshold,
        shutdown_wait_duration_ms,
        watchdog_timeout_ms,
        led_brightness,
    );
    info!("Runtime configuration updated");
}

#[task]
pub async fn config_manager_task() {
    info!("Initializing config manager task");

    // Flash and config manager are initialized in the main function to ensure
    // their availability for other tasks before this task runs.

    info!("Config manager task started");

    let receiver = CONFIG_MANAGER_EVENT_CHANNEL.receiver();

    loop {
        let event = receiver.receive().await;
        debug!("Received config manager event: {:?}", event);

        let mut config_manager = CONFIG_MANAGER.get().await.lock().await;

        match event {
            ConfigManagerEvents::VscapPowerOnThreshold(value) => {
                config_manager
                    .set(VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::VinPowerThreshold(value) => {
                config_manager
                    .set(VIN_POWER_THRESHOLD_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::ShutdownWaitDurationMs(value) => {
                config_manager
                    .set(SHUTDOWN_WAIT_DURATION_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::WatchdogTimeoutMs(value) => {
                config_manager
                    .set(HOST_WATCHDOG_TIMEOUT_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::LedBrightness(value) => {
                config_manager
                    .set(LED_BRIGHTNESS_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
        }

    }
}
