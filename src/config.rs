

pub const VSCAP_POWER_ON: f32 = 6.0; // V
pub const VSCAP_POWER_OFF: f32 = 5.0; // V
pub const VSCAP_MAX_ALARM: f32 = 10.5; // V; Voltage should never exceed this value
pub const VSCAP_MAX: f32 = 11.0; // V; Maximum voltage for Vscap

pub const VIN_OFF: f32 = 9.0; // V
pub const VIN_MAX: f32 = 33.0; // V

pub const SHUTDOWN_WAIT_DURATION_MS: u32 = 60_000; // ms

// how long to stay in off state until restarting
pub const OFF_STATE_DURATION_MS: u32 = 5000; // ms

// how long to keep VEN low in the event of watchdog reboot
pub const WATCHDOG_REBOOT_DURATION_MS: u32 = 2000; // ms
