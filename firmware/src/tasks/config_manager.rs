use defmt::{debug, error, info};
use embassy_executor::task;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel;
use embassy_sync::mutex::Mutex;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{SerializationError, fetch_item, remove_item, store_item};
use serde::{Deserialize, Serialize};

use crate::flash_layout::get_bootloader_appdata_range;
use crate::{MFlashType, config::*};

// Define a comprehensive error type
#[derive(Debug)]
pub enum ConfigError {
    // Flash operation errors
    Flash(()),
    // Other storage errors
    Storage,
}

impl From<embassy_rp::flash::Error> for ConfigError {
    fn from(_: embassy_rp::flash::Error) -> Self {
        ConfigError::Flash(())
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
    VscapPowerOffThreshold(f32),
    VinPowerThreshold(f32),
    ShutdownWaitDurationMs(u32),
    SoloDepletingTimeoutMs(u32),
    WatchdogTimeoutMs(u16),
    LedBrightness(u8),
    VscapCorrectionScale(f32),
    VinCorrectionScale(f32),
    IinCorrectionScale(f32),
    AutoRestart(bool),
}

pub type ConfigManagerChannelType =
    channel::Channel<CriticalSectionRawMutex, ConfigManagerEvents, 8>;
pub static CONFIG_MANAGER_EVENT_CHANNEL: ConfigManagerChannelType = channel::Channel::new();

// Configuration manager using sequential-storage
pub struct ConfigManager {
    flash: &'static MFlashType<'static>,
    data_buffer: [u8; 128],
}

impl ConfigManager {
    fn new(flash: &'static MFlashType<'static>) -> Self {
        let data_buffer = [0u8; 128];

        Self { flash, data_buffer }
    }

    /// Store a serializable value
    pub async fn set<T>(&mut self, key: u16, value: &T) -> Result<(), ConfigError>
    where
        T: for<'de> Deserialize<'de> + Serialize + for<'b> sequential_storage::map::Value<'b>,
    {
        debug!("Storing item with key: {}", key);

        let mut flash = self.flash.lock().await;

        let result = store_item(
            &mut *flash,
            get_bootloader_appdata_range(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
            value,
        )
        .await;

        match result {
            Ok(_) => {
                debug!("Item stored successfully with key: {}", key);
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to store item with key: {}: {}",
                    key,
                    defmt::Debug2Format(&e)
                );
                Err(ConfigError::from(e))
            }
        }
    }

    // Retrieve a value or None if not found
    // Modified to remove the lifetime dependency on self
    pub async fn get<T>(&mut self, key: u16) -> Result<Option<T>, ConfigError>
    where
        T: for<'de> Deserialize<'de> + Serialize + for<'b> sequential_storage::map::Value<'b>,
    {
        debug!("Fetching item with key: {}", key);

        let mut flash = self.flash.lock().await;

        let result = fetch_item(
            &mut *flash,
            get_bootloader_appdata_range(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await;

        match result {
            Ok(Some(value)) => {
                debug!("Item fetched successfully with key: {}", key);
                Ok(Some(value))
            }
            Ok(None) => {
                debug!("No item found with key: {}", key);
                Ok(None)
            }
            Err(e) => {
                error!(
                    "Failed to fetch item with key: {}: {}",
                    key,
                    defmt::Debug2Format(&e)
                );
                Err(ConfigError::from(e))
            }
        }
    }

    // Check if a key exists
    #[allow(dead_code)]
    pub async fn contains_key(&mut self, key: u16) -> Result<bool, ConfigError> {
        let mut flash = self.flash.lock().await;
        let result = fetch_item::<u16, Option<bool>, _>(
            &mut *flash,
            get_bootloader_appdata_range(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await
        .map_err(ConfigError::from)?;

        Ok(result.is_some())
    }

    // Remove a key
    #[allow(dead_code)]
    pub async fn remove(&mut self, key: u16) -> Result<(), ConfigError> {
        let mut flash = self.flash.lock().await;
        remove_item(
            &mut *flash,
            get_bootloader_appdata_range(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
        )
        .await
        .map_err(ConfigError::from)
    }

    // Erase the entire flash range
    #[allow(dead_code)]
    pub async fn erase(&mut self) -> Result<(), ConfigError> {
        let mut flash = self.flash.lock().await;
        sequential_storage::erase_all(&mut *flash, get_bootloader_appdata_range())
            .await
            .map_err(ConfigError::from)
    }
}

/// Runtime configuration values, read from the flash storage and stored here
/// to prevent multiple reads from the flash.
struct RuntimeConfig {
    pub vscap_power_on_threshold: f32,
    pub vscap_power_off_threshold: f32,
    pub vin_power_threshold: f32,
    pub shutdown_wait_duration_ms: u32,
    pub solo_depleting_timeout_ms: u32,
    pub watchdog_timeout_ms: u16,
    pub led_brightness: u8,
    pub vin_correction_scale: f32,
    pub vscap_correction_scale: f32,
    pub iin_correction_scale: f32,
    pub auto_restart: bool,
}

impl RuntimeConfig {
    const fn new(
        vscap_power_on_threshold: f32,
        vscap_power_off_threshold: f32,
        vin_power_threshold: f32,
        shutdown_wait_duration_ms: u32,
        solo_depleting_timeout_ms: u32,
        watchdog_timeout_ms: u16,
        led_brightness: u8,
        vin_correction_scale: f32,
        vscap_correction_scale: f32,
        iin_correction_scale: f32,
        auto_restart: bool,
    ) -> Self {
        RuntimeConfig {
            vscap_power_on_threshold,
            vscap_power_off_threshold,
            vin_power_threshold,
            shutdown_wait_duration_ms,
            solo_depleting_timeout_ms,
            watchdog_timeout_ms,
            led_brightness,
            vin_correction_scale,
            vscap_correction_scale,
            iin_correction_scale,
            auto_restart,
        }
    }
}

static RUNTIME_CONFIG: Mutex<CriticalSectionRawMutex, RuntimeConfig> =
    Mutex::new(RuntimeConfig::new(
        DEFAULT_VSCAP_POWER_ON_THRESHOLD,
        DEFAULT_VSCAP_POWER_OFF_THRESHOLD,
        DEFAULT_VIN_POWER_THRESHOLD,
        DEFAULT_SHUTDOWN_WAIT_DURATION_MS,
        DEFAULT_SOLO_DEPLETING_TIMEOUT_MS,
        HOST_WATCHDOG_DEFAULT_TIMEOUT_MS,
        DEFAULT_LED_BRIGHTNESS,
        DEFAULT_VIN_CORRECTION_SCALE,
        DEFAULT_VSCAP_CORRECTION_SCALE,
        DEFAULT_IIN_CORRECTION_SCALE,
        DEFAULT_AUTO_RESTART,
    ));

pub async fn get_vscap_power_on_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_on_threshold
}
pub async fn get_vscap_power_off_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vscap_power_off_threshold
}
#[allow(dead_code)]
pub async fn get_vin_power_threshold() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vin_power_threshold
}
#[allow(dead_code)]
pub async fn get_shutdown_wait_duration_ms() -> u32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.shutdown_wait_duration_ms
}
pub async fn get_solo_depleting_timeout_ms() -> u32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.solo_depleting_timeout_ms
}
#[allow(dead_code)]
pub async fn get_watchdog_timeout_ms() -> u16 {
    let config = RUNTIME_CONFIG.lock().await;
    config.watchdog_timeout_ms
}
pub async fn get_led_brightness() -> u8 {
    let config = RUNTIME_CONFIG.lock().await;
    config.led_brightness
}
pub async fn get_vin_correction_scale() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vin_correction_scale
}
pub async fn get_vscap_correction_scale() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.vscap_correction_scale
}
pub async fn get_iin_correction_scale() -> f32 {
    let config = RUNTIME_CONFIG.lock().await;
    config.iin_correction_scale
}
pub async fn get_auto_restart() -> bool {
    let config = RUNTIME_CONFIG.lock().await;
    config.auto_restart
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
        .send(ConfigManagerEvents::VscapPowerOffThreshold(value))
        .await;
}
#[allow(dead_code)]
pub async fn set_vin_power_threshold(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vin_power_threshold = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VinPowerThreshold(value))
        .await;
}
#[allow(dead_code)]
pub async fn set_shutdown_wait_duration_ms(value: u32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.shutdown_wait_duration_ms = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::ShutdownWaitDurationMs(value))
        .await;
}
pub async fn set_solo_depleting_timeout_ms(value: u32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.solo_depleting_timeout_ms = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::SoloDepletingTimeoutMs(value))
        .await;
}
#[allow(dead_code)]
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
pub async fn set_vin_correction_scale(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vin_correction_scale = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VinCorrectionScale(value))
        .await;
}
pub async fn set_vscap_correction_scale(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.vscap_correction_scale = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::VscapCorrectionScale(value))
        .await;
}
pub async fn set_iin_correction_scale(value: f32) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.iin_correction_scale = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::IinCorrectionScale(value))
        .await;
}
pub async fn set_auto_restart(value: bool) {
    let mut config = RUNTIME_CONFIG.lock().await;
    config.auto_restart = value;
    CONFIG_MANAGER_EVENT_CHANNEL
        .send(ConfigManagerEvents::AutoRestart(value))
        .await;
}

pub async fn init_config_manager(
    flash: &'static MFlashType<'static>,
) -> Mutex<CriticalSectionRawMutex, ConfigManager> {
    let config_manager_mutex =
        Mutex::<CriticalSectionRawMutex, ConfigManager>::new(ConfigManager::new(flash));
    info!("Config manager initialized");

    {
        let mut config_manager = config_manager_mutex.lock().await;

        let vscap_power_on_threshold = config_manager
            .get::<f32>(VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_VSCAP_POWER_ON_THRESHOLD);
        debug!(
            "Received vscap power on threshold: {}",
            vscap_power_on_threshold
        );
        let vscap_power_off_threshold = config_manager
            .get::<f32>(VSCAP_POWER_OFF_THRESHOLD_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_VSCAP_POWER_OFF_THRESHOLD);
        debug!(
            "Received vscap power off threshold: {}",
            vscap_power_off_threshold
        );
        let vin_power_threshold = config_manager
            .get::<f32>(VIN_POWER_THRESHOLD_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_VIN_POWER_THRESHOLD);
        debug!("Received vin power threshold: {}", vin_power_threshold);
        let shutdown_wait_duration_ms = config_manager
            .get::<u32>(SHUTDOWN_WAIT_DURATION_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_SHUTDOWN_WAIT_DURATION_MS);
        debug!(
            "Received shutdown wait duration: {}",
            shutdown_wait_duration_ms
        );
        let solo_depleting_timeout_ms = config_manager
            .get::<u32>(SOLO_DEPLETING_TIMEOUT_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_SOLO_DEPLETING_TIMEOUT_MS);
        debug!(
            "Received solo depleting timeout: {}",
            solo_depleting_timeout_ms
        );
        let watchdog_timeout_ms = config_manager
            .get::<u16>(HOST_WATCHDOG_TIMEOUT_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(HOST_WATCHDOG_DEFAULT_TIMEOUT_MS);
        debug!("Received watchdog timeout: {}", watchdog_timeout_ms);
        let led_brightness = config_manager
            .get::<u8>(LED_BRIGHTNESS_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_LED_BRIGHTNESS);
        debug!("Received led brightness: {}", led_brightness);
        let auto_restart = config_manager
            .get::<bool>(AUTO_RESTART_CONFIG_KEY)
            .await
            .unwrap_or(None)
            .unwrap_or(DEFAULT_AUTO_RESTART);
        debug!("Received auto restart: {}", auto_restart);

        let mut runtime_config = RUNTIME_CONFIG.lock().await;
        runtime_config.vscap_power_on_threshold = vscap_power_on_threshold;
        runtime_config.vscap_power_off_threshold = vscap_power_off_threshold;
        runtime_config.vin_power_threshold = vin_power_threshold;
        runtime_config.shutdown_wait_duration_ms = shutdown_wait_duration_ms;
        runtime_config.solo_depleting_timeout_ms = solo_depleting_timeout_ms;
        runtime_config.watchdog_timeout_ms = watchdog_timeout_ms;
        runtime_config.led_brightness = led_brightness;
        runtime_config.auto_restart = auto_restart;
    }
    info!("Runtime configuration updated");
    config_manager_mutex
}

#[task]
pub async fn config_manager_task(flash: &'static MFlashType<'static>) {
    info!("Initializing config manager task");

    let config_manager_mutex = init_config_manager(flash).await;

    // Flash and config manager are initialized in the main function to ensure
    // their availability for other tasks before this task runs.

    info!("Config manager task started");

    let receiver = CONFIG_MANAGER_EVENT_CHANNEL.receiver();

    loop {
        let event = receiver.receive().await;
        debug!("Received config manager event: {:?}", event);

        let mut config_manager = config_manager_mutex.lock().await;

        match event {
            ConfigManagerEvents::VscapPowerOnThreshold(value) => {
                config_manager
                    .set(VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::VscapPowerOffThreshold(value) => {
                config_manager
                    .set(VSCAP_POWER_OFF_THRESHOLD_CONFIG_KEY, &value)
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
            ConfigManagerEvents::SoloDepletingTimeoutMs(value) => {
                config_manager
                    .set(SOLO_DEPLETING_TIMEOUT_CONFIG_KEY, &value)
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
            ConfigManagerEvents::VinCorrectionScale(value) => {
                config_manager
                    .set(VIN_CORRECTION_SCALE_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::VscapCorrectionScale(value) => {
                config_manager
                    .set(VSCAP_CORRECTION_SCALE_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::IinCorrectionScale(value) => {
                config_manager
                    .set(IIN_CORRECTION_SCALE_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
            ConfigManagerEvents::AutoRestart(value) => {
                config_manager
                    .set(AUTO_RESTART_CONFIG_KEY, &value)
                    .await
                    .unwrap();
            }
        }
    }
}
