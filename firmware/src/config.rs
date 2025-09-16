pub const I2C_ADDR: u8 = 0x6d; // I2C address for the device secondary interface

pub const VSCAP_MAX_ALARM: f32 = 10.5; // V; Voltage should never exceed this value
pub const VSCAP_MAX_VALUE: f32 = 11.0; // V; Maximum voltage for Vscap

pub const DEFAULT_VSCAP_CORRECTION_SCALE: f32 = 1.059; // Default correction scale for Vscap
pub const VSCAP_CORRECTION_SCALE_CONFIG_KEY: u16 = 0x100a; // Key for Vscap correction scale in the config

// Default values for power thresholds
pub const DEFAULT_VSCAP_POWER_ON_THRESHOLD: f32 = 8.0; // V
pub const VSCAP_POWER_ON_THRESHOLD_CONFIG_KEY: u16 = 0x1002;
pub const DEFAULT_VSCAP_POWER_OFF_THRESHOLD: f32 = 5.5; // V
pub const VSCAP_POWER_OFF_THRESHOLD_CONFIG_KEY: u16 = 0x1003;

pub const DEFAULT_VIN_POWER_THRESHOLD: f32 = 9.0; // V
pub const VIN_POWER_THRESHOLD_CONFIG_KEY: u16 = 0x1004;
pub const VIN_MAX_VALUE: f32 = 40.0; // V
pub const DEFAULT_VIN_CORRECTION_SCALE: f32 = 1.015; // Default correction scale for VIN
pub const VIN_CORRECTION_SCALE_CONFIG_KEY: u16 = 0x1008; // Key for VIN correction scale in the config

pub const IIN_MAX_VALUE: f32 = 3.3; // V; Maximum voltage for Iin
// Default correction scale for Iin. The default value is experimentally determined to correct
// scaling error present in the 0.4.0 hardware.
pub const DEFAULT_IIN_CORRECTION_SCALE: f32 = 0.811_533_1;
pub const IIN_CORRECTION_SCALE_CONFIG_KEY: u16 = 0x1009;

// Time to wait for device to shut down gracefully.
// Once this time is reached, the device will forcefully shut down.
pub const DEFAULT_SHUTDOWN_WAIT_DURATION_MS: u32 = 60_000; // ms
pub const SHUTDOWN_WAIT_DURATION_CONFIG_KEY: u16 = 0x1005;

// Time to wait for the device to start shutting down once the power is cut.
pub const DEFAULT_SOLO_BLACKOUT_TIMEOUT_MS: u32 = 5_000; // ms
pub const SOLO_BLACKOUT_TIMEOUT_CONFIG_KEY: u16 = 0x1007;

// how long to stay in off state until restarting
pub const OFF_STATE_DURATION_MS: u32 = 5000; // ms

// Default timeout for the host watchdog. If the watchdog is not reset within this time,
// the device will reboot.
pub const HOST_WATCHDOG_DEFAULT_TIMEOUT_MS: u16 = 10_000; // ms
pub const HOST_WATCHDOG_TIMEOUT_CONFIG_KEY: u16 = 0x1006; // Key for the watchdog timeout in the config

// how long to stay in the watchdog alert state before rebooting
pub const HOST_WATCHDOG_REBOOT_DURATION_MS: u32 = 5000; // ms

pub const FLASH_SIZE: usize = 4 * 1024 * 1024;

pub const LED_BRIGHTNESS_CONFIG_KEY: u16 = 0x1001;
pub const DEFAULT_LED_BRIGHTNESS: u8 = 0x30; // Default brightness value

pub const AUTO_RESTART_CONFIG_KEY: u16 = 0x100b;
pub const DEFAULT_AUTO_RESTART: bool = true; // Default: auto restart enabled

pub const HARDWARE_VERSION_CONFIG_KEY: u16 = 0x100c;
pub const DEFAULT_HARDWARE_VERSION: u32 = 0xffff; // Default: return 0xFFFF if not found

pub const MIN_TEMPERATURE_VALUE: f32 = 273.15 - 40.0; // Minimum temperature value
pub const MAX_TEMPERATURE_VALUE: f32 = 273.15 + 100.0; // Maximum temperature value

pub const MAX_FLASH_WRITE_QUEUE_DEPTH: usize = 4; // Adjust based on available RAM
pub const FLASH_ERASE_BLOCK_SIZE: usize = 4096;
pub const FLASH_WRITE_BLOCK_SIZE: usize = 4096;

pub const FIRMWARE_MARK_BOOTED_DELAY_MS: u32 = 30_000; // Delay before marking firmware as booted

pub const FW_VERSION_STR: &str = "3.1.2-a1";

// Parse version strings into byte arrays
// The version format is [major, minor, patch, alpha], where alpha is 0xff
// for stable releases and a running number for alpha releases.

macro_rules! parse_version {
    ($ver:expr) => {{
        // Accepts "x.y.z" or "x.y.z-aN"
        const fn parse(ver: &str) -> [u8; 4] {
            let bytes = ver.as_bytes();
            let mut major = 0u8;
            let mut minor = 0u8;
            let mut patch = 0u8;
            let mut alpha = 0xffu8;
            let mut i = 0;
            // Parse major
            while i < bytes.len() && bytes[i] != b'.' {
                major = major * 10 + (bytes[i] - b'0');
                i += 1;
            }
            i += 1; // skip '.'
            // Parse minor
            while i < bytes.len() && bytes[i] != b'.' {
                minor = minor * 10 + (bytes[i] - b'0');
                i += 1;
            }
            i += 1; // skip '.'
            // Parse patch
            while i < bytes.len() && i < bytes.len() && bytes[i] != b'-' {
                patch = patch * 10 + (bytes[i] - b'0');
                i += 1;
            }
            // Parse alpha if present
            if i < bytes.len() && bytes[i] == b'-' {
                i += 1; // skip '-'
                if i + 1 < bytes.len() && bytes[i] == b'a' {
                    i += 1; // skip 'a'
                    let mut a = 0u8;
                    while i < bytes.len() {
                        a = a * 10 + (bytes[i] - b'0');
                        i += 1;
                    }
                    alpha = a;
                }
            }
            [major, minor, patch, alpha]
        }
        parse($ver)
    }};
}

pub const FW_VERSION: [u8; 4] = parse_version!(FW_VERSION_STR);
