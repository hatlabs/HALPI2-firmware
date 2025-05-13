use alloc::string::{String, ToString};
use core::ops::Range;
use defmt::{debug, info};
use embassy_rp::flash::{Async, Flash};
use embassy_rp::peripherals::FLASH;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{SerializationError, fetch_item, remove_item, store_item};
use serde::{Deserialize, Serialize};

use crate::config::{FLASH_CONFIG_OFFSET, FLASH_CONFIG_SIZE, FLASH_SIZE};

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

pub async fn init_config_manager(
    flash: embassy_rp::flash::Flash<'static, FLASH, Async, FLASH_SIZE>,
) {
    let config_manager = ConfigManager::new(flash, FLASH_CONFIG_OFFSET, FLASH_CONFIG_SIZE);
    if CONFIG_MANAGER.init(Mutex::new(config_manager)).is_err() {
        // Handle the error appropriately, e.g., log it or panic
        panic!("Failed to initialize CONFIG_MANAGER");
    }
    info!("Config manager initialized");
}
