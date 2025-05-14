pub const VSCAP_MAX_ALARM: f32 = 10.5; // V; Voltage should never exceed this value
pub const VSCAP_MAX_VALUE: f32 = 11.0; // V; Maximum voltage for Vscap

// Default values for power thresholds
pub const DEFAULT_VSCAP_POWER_ON_THRESHOLD: f32 = 8.0; // V
pub const VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY: u16 = 0x1002;
pub const DEFAULT_VSCAP_POWER_OFF_THRESHOLD: f32 = 5.5; // V
pub const VSCAP_POWER_OFF_THRESHOLD_CONFIG_KEY: u16 = 0x1003;

pub const DEFAULT_VIN_POWER_THRESHOLD: f32 = 9.0; // V
pub const VIN_POWER_THRESHOLD_CONFIG_KEY: u16 = 0x1004;
pub const VIN_MAX_VALUE: f32 = 33.0; // V

pub const IIN_MAX_VALUE: f32 = 3.3; // V; Maximum voltage for Iin

pub const DEFAULT_SHUTDOWN_WAIT_DURATION_MS: u32 = 60_000; // ms
pub const SHUTDOWN_WAIT_DURATION_CONFIG_KEY: u16 = 0x1005;

// how long to stay in off state until restarting
pub const OFF_STATE_DURATION_MS: u32 = 5000; // ms

pub const HOST_WATCHDOG_DEFAULT_TIMEOUT_MS: u16 = 10_000; // ms
pub const HOST_WATCHDOG_TIMEOUT_CONFIG_KEY: u16 = 0x1006; // Key for the watchdog timeout in the config

// how long to keep VEN low in the event of watchdog reboot
pub const HOST_WATCHDOG_REBOOT_DURATION_MS: u32 = 2000; // ms

pub const FLASH_SIZE: usize = 4 * 1024 * 1024;
pub const FLASH_CONFIG_OFFSET: u32 = 256 * 1024; // Offset for the config data in flash
pub const FLASH_CONFIG_SIZE: u32 = 64 * 1024; // Size of the config data in flash

pub const LED_BRIGHTNESS_CONFIG_KEY: u16 = 0x1001;
pub const DEFAULT_LED_BRIGHTNESS: u8 = 0xFF; // Default brightness value

pub const MIN_TEMPERATURE_VALUE: f32 = 274.15 - 40.0; // Minimum temperature value
pub const MAX_TEMPERATURE_VALUE: f32 = 274.15 + 100.0; // Maximum temperature value
