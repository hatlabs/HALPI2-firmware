use crate::led_patterns::{get_state_pattern, get_vscap_alarm_pattern};
use crate::tasks::config_manager::{get_auto_restart, get_shutdown_wait_duration_ms, get_solo_depleting_timeout_ms, get_vscap_power_on_threshold, usb_power_on, usb_power_off};
use crate::tasks::led_blinker::{LED_BLINKER_EVENT_CHANNEL, LEDBlinkerEvents};
use crate::tasks::power_button::{POWER_BUTTON_EVENT_CHANNEL, PowerButtonEvents};
use alloc::vec::Vec;
use core::fmt::Debug;
use cortex_m::peripheral::SCB;
use defmt::*;
use embassy_executor::task;
use embassy_rp::gpio::{Level, Output};
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::once_lock::OnceLock;
use embassy_time::{Duration, Instant, Ticker};
use statig::prelude::*;

use crate::config::*;
use crate::config_resources::StateMachineOutputResources;
use crate::tasks::gpio_input::INPUTS;

use super::led_blinker::LEDBlinkerChannelType;
use super::power_button::PowerButtonChannelType;

/// Helper function to check if VIN power is available
async fn is_vin_power_available() -> bool {
    let inputs = INPUTS.lock().await;
    let vin_power = inputs.vin > DEFAULT_VIN_POWER_THRESHOLD;
    drop(inputs);
    vin_power
}

/// Helper function to check vscap voltage and return (voltage, is_above_threshold)
async fn get_vscap_status() -> (f32, bool) {
    let inputs = INPUTS.lock().await;
    let vscap = inputs.vscap;
    let is_alarm = vscap > VSCAP_MAX_ALARM;
    drop(inputs);
    (vscap, is_alarm)
}


/// Events that can be sent to the state machine from external tasks via channel
#[allow(dead_code)]
pub enum StateMachineEvents {
    /// Request graceful shutdown sequence
    Shutdown,
    /// Request transition to standby mode (low power)
    StandbyShutdown,
    /// Force immediate power off
    Off,
    /// Configure host watchdog timeout in milliseconds (0 = disabled)
    SetHostWatchdogTimeout(u16),
    /// Host system sends keepalive ping to reset watchdog timer
    HostWatchdogPing,
    /// Physical power button was pressed
    PowerButtonPress,
}

pub type StateMachineChannelType =
    channel::Channel<CriticalSectionRawMutex, StateMachineEvents, 16>;
pub static STATE_MACHINE_EVENT_CHANNEL: StateMachineChannelType = channel::Channel::new();

/// Internal events used by the state machine for state transitions
/// These are generated automatically based on hardware inputs and timers
///
/// # Naming Conventions
/// - Hardware events use descriptive names (ComputeModuleOn/Off)
/// - Timer events use "Tick" for regular intervals and VIN level checks
/// - User/host actions use descriptive verbs (Shutdown, WatchdogPing)
/// - Alarm events use specific names for safety-critical conditions (SupercapOvervoltage)
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Regular timer tick (50ms intervals) for timeout and periodic checks
    Tick,
    /// Supercapacitor voltage exceeded maximum safe threshold (10.5V) - overvoltage alarm
    SupercapOvervoltage,
    /// Compute Module 5 powered on (3.3V rail active)
    ComputeModuleOn,
    /// Compute Module 5 powered off (3.3V rail inactive)
    ComputeModuleOff,
    /// Request graceful shutdown sequence
    Shutdown,
    /// Request transition to standby mode (low power)
    StandbyShutdown,
    /// Force immediate power off
    Off,
    /// Configure host watchdog timeout in milliseconds (0 = disabled)
    SetWatchdogTimeout(u16),
    /// Host system sends keepalive ping to reset watchdog timer
    WatchdogPing,
    /// Physical power button was pressed
    PowerButtonPress,
}

/// GPIO outputs that are controlled by the state machine task.
pub struct Outputs {
    pub ven: Output<'static>,
    pub pcie_sleep: Output<'static>,
}

impl Outputs {
    fn new(resources: StateMachineOutputResources) -> Self {
        Outputs {
            ven: Output::new(resources.ven, Level::Low),
            pcie_sleep: Output::new(resources.pcie_sleep, Level::Low),
        }
    }

    fn power_on(&mut self) {
        self.ven.set_high();
        self.pcie_sleep.set_low();
    }

    fn power_off(&mut self) {
        self.ven.set_low();
        self.pcie_sleep.set_high();
    }
}

pub struct Context {
    pub outputs: Outputs,
    pub power_button_channel: &'static PowerButtonChannelType,
    pub led_blinker_channel: &'static LEDBlinkerChannelType,
    pub host_watchdog_timeout_ms: u16,
    pub host_watchdog_last_ping: Instant,
    pub vscap_alarm_active: bool,
}

impl Context {
    pub fn new(
        outputs: Outputs,
        power_button_channel: &'static PowerButtonChannelType,
        led_blinker_channel: &'static LEDBlinkerChannelType,
        host_watchdog_timeout_ms: u16,
    ) -> Self {
        Context {
            outputs,
            power_button_channel,
            led_blinker_channel,
            host_watchdog_timeout_ms,
            host_watchdog_last_ping: Instant::now(),
            vscap_alarm_active: false,
        }
    }

    async fn set_led_pattern(&self, state: &State) {
        let _ = self
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_state_pattern(state)))
            .await;
    }

    async fn set_alarm_led_pattern(&self) {
        let _ = self
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_vscap_alarm_pattern()))
            .await;
    }

    async fn send_power_button_event(&self, event: PowerButtonEvents) {
        let _ = self.power_button_channel.send(event).await;
    }
}

static STATE_MACHINE_STATE: OnceLock<Mutex<NoopRawMutex, State>> = OnceLock::new();

pub async fn get_state_machine_state() -> State {
    *STATE_MACHINE_STATE.get().await.lock().await
}

pub fn state_as_str(state: &State) -> &'static str {
    match state {
        State::PowerOff {} => "PowerOff",
        State::OffCharging {} => "OffCharging",
        State::SystemStartup {} => "SystemStartup",
        State::OperationalSolo {} => "OperationalSolo",
        State::OperationalCoOp {} => "OperationalCoOp",
        State::BlackoutSolo { .. } => "BlackoutSolo",
        State::BlackoutCoOp { .. } => "BlackoutCoOp",
        State::BlackoutShutdown { .. } => "BlackoutShutdown",
        State::ManualShutdown { .. } => "ManualShutdown",
        State::PoweredDownBlackout { .. } => "PoweredDownBlackout",
        State::PoweredDownManual { .. } => "PoweredDownManual",
        State::HostUnresponsive { .. } => "HostUnresponsive",
        State::EnteringStandby { .. } => "EnteringStandby",
        State::Standby {} => "Standby",
    }
}

pub fn state_as_u8(state: &State) -> u8 {
    // Note: the state numbering is part of the I2C API. Any new states
    // must be added with a unique number.
    match state {
        State::PowerOff {} => 0,
        State::OffCharging {} => 1,
        State::SystemStartup {} => 2,
        State::OperationalSolo {} => 3,
        State::OperationalCoOp {} => 4,
        State::BlackoutSolo { .. } => 5,
        State::BlackoutCoOp { .. } => 6,
        State::BlackoutShutdown { .. } => 7,
        State::ManualShutdown { .. } => 8,
        State::PoweredDownBlackout { .. } => 9,
        State::PoweredDownManual { .. } => 10,
        State::HostUnresponsive { .. } => 11,
        State::EnteringStandby { .. } => 12,
        State::Standby {} => 13,
    }
}

pub async fn record_state_machine_state(state: &State) {
    *STATE_MACHINE_STATE.get().await.lock().await = *state;
}

#[derive(Debug, Default)]
pub struct HalpiStateMachine {}

/// HALPI2 Power Management State Machine
///
/// This state machine manages the power states and operational modes of the HALPI2 system,
/// which provides backup power via supercapacitor during external power loss.
///
/// # State Hierarchy
///
/// ```
/// PowerOff ──ExternalPowerOn──> OffCharging ──(vscap>=threshold)──> SystemStartup ──ComputeModuleOn──> [PoweredOn]
///    ^                              ^                                    ^                                 │
///    │                              │                                    │                                 │
///    └─────ExternalPowerOff─────────┴─────ExternalPowerOff───────────────┴─────────────────────────────────┘
///
/// [PoweredOn] (superstate)
/// ├── [Operational] (superstate)
/// │   ├── OperationalSolo
/// │   │   └── ExternalPowerOff ──> BlackoutSolo
/// │   └── OperationalCoOp
/// │       └── ExternalPowerOff ──> BlackoutCoOp
/// ├── [Blackout] (superstate)
/// │   ├── BlackoutSolo ──ExternalPowerOn──> OperationalSolo/CoOp (based on watchdog setting)
/// │   ├── BlackoutCoOp ──ExternalPowerOn──> OperationalSolo/CoOp (based on watchdog setting)
/// │   └── Timeout/Shutdown ──> BlackoutShutdown ──> PoweredDownBlackout
/// ├── HostUnresponsive (host watchdog timeout)
/// │   ├── WatchdogPing ──> Operational(cooperative)
/// │   └── Timeout ──> PoweredDownBlackout
/// └── EnteringStandby ──ComputeModuleOff──> Standby ──ComputeModuleOn──> Operational(solo)
///
/// PoweredDownBlackout ──[always restart after timeout]──> System Reset
/// PoweredDownManual ──[restart if auto_restart enabled]──> System Reset
/// ManualShutdown ──ComputeModuleOff/Timeout──> PoweredDownManual
/// ```
///
/// # Key Features
///
/// - **Backup Power**: Automatic transition to supercapacitor power during external power loss
/// - **Watchdog Monitoring**: Optional host watchdog with configurable timeout and recovery
/// - **Graceful Shutdown**: Configurable shutdown timeouts to prevent data corruption
/// - **Overvoltage Protection**: Persistent alarm at 10.5V supercap voltage with LED warning
/// - **Dual Operating Modes**: Solo (independent) and Cooperative (host-dependent) operation
/// - **Standby Mode**: Low-power state with wake capability
/// - **Auto-restart**: Configurable restart behavior for different shutdown scenarios
///
/// # Operating Modes
///
/// - **Solo Mode**: System operates independently without host watchdog
/// - **Cooperative Mode**: Host must send periodic pings; watchdog triggers recovery if host fails
///
/// # Safety Features
///
/// - Configurable timeouts for supercap depletion (default 30s in solo mode)
/// - Persistent overvoltage alarm (never auto-clears, requires reset)
/// - Graceful shutdown sequences to prevent data corruption
/// - Automatic restart on power events for high availability
///
/// # Configuration
///
/// - Solo depleting timeout: Configurable via I2C command 0x19
/// - Shutdown wait duration: Configurable via flash storage
/// - Auto-restart behavior: Configurable via flash storage
/// - Host watchdog timeout: Configurable via I2C commands

#[state_machine(
    initial = "State::power_off()",
    before_transition = "Self::before_transition",
    state(derive(Copy, Clone, Debug)),
    superstate(derive(Copy, Clone, Debug))
)]
impl HalpiStateMachine {
    async fn before_transition(&mut self, source: &State, target: &State) {
        info!(
            "Transitioning from {:?} to {:?}",
            defmt::Debug2Format(source),
            defmt::Debug2Format(target)
        );
    }

    /// Initial state: System is completely off with no external power
    ///
    /// Hardware state:
    /// - All power rails disabled (VEN=low)
    /// - USB ports disabled
    /// - PCIe in sleep mode
    ///
    /// Transitions:
    /// - Tick (when VIN > threshold) -> OffCharging (external power applied)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_power_off")]
    async fn power_off(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power is available
                if is_vin_power_available().await {
                    Transition(State::off_charging())
                } else {
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_power_off(&mut self, context: &mut Context) {
        context.outputs.power_off();
        usb_power_off().await;
    }

    /// System is off but supercapacitor is charging from external power
    ///
    /// Hardware state:
    /// - External power (VIN) available but system not yet powered
    /// - Supercapacitor charging up to operational voltage
    /// - LED shows charging pattern
    ///
    /// Transitions:
    /// - Tick (when vscap >= threshold) -> SystemStartup (supercap charged enough to boot)
    /// - Tick (when VIN <= threshold) -> PowerOff (external power removed)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off_charging")]
    async fn off_charging(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power is still available
                if !is_vin_power_available().await {
                    return Transition(State::power_off());
                }

                // Check if supercap voltage is sufficient for system startup
                let inputs = INPUTS.lock().await;
                let vscap_threshold = get_vscap_power_on_threshold().await;
                if inputs.vscap >= vscap_threshold {
                    drop(inputs);
                    Transition(State::system_startup())
                } else {
                    drop(inputs);
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_off_charging(&mut self, context: &mut Context) {
        context.set_led_pattern(&State::off_charging()).await;
    }

    /// System is powering on and waiting for Compute Module to initialize
    ///
    /// Hardware state:
    /// - 5V rail powered (VEN=high)
    /// - USB ports enabled
    /// - PCIe active
    /// - Waiting for CM5 3.3V rail to stabilize
    /// - LED shows boot pattern
    ///
    /// Transitions:
    /// - ComputeModuleOn -> Operational(solo) (CM5 successfully powered up)
    /// - Tick (when VIN <= threshold) -> PowerOff (power lost during boot)
    #[state(entry_action = "enter_system_startup")]
    async fn system_startup(event: &Event) -> Outcome<State> {
        match event {
            Event::ComputeModuleOn => Transition(State::operational_solo()), // Start in solo mode
            Event::Tick => {
                // Check if external power is still available
                if !is_vin_power_available().await {
                    Transition(State::power_off())
                } else {
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_system_startup(context: &mut Context) {
        context.outputs.power_on();
        usb_power_on().await;
        context.set_led_pattern(&State::system_startup()).await;
    }

    /// Superstate for all situations where the system is powered on and running
    ///
    /// This superstate handles common events for all powered states:
    /// - SupercapOvervoltage: Activates persistent red LED warning for overvoltage (>10.5V)
    /// - ComputeModuleOff: CM5 has powered itself off abruptly - follow its lead
    /// - Off: Force immediate shutdown
    /// - WatchdogPing: Updates host watchdog timer
    ///
    /// Child states: Operational, Blackout, HostUnresponsive
    #[allow(unused_variables)]
    #[superstate]
    async fn powered_on(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::ComputeModuleOff => Transition(State::powered_down_manual(Instant::now())), // CM5 powered itself off - command-based shutdown
            Event::Off => Transition(State::powered_down_manual(Instant::now())), // Force immediate shutdown - command-based
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Handled
            }
            Event::SupercapOvervoltage => {
                warn!("Supercapacitor overvoltage alarm activated!");
                context.vscap_alarm_active = true;
                // Override LED pattern with alarm pattern
                context.set_alarm_led_pattern().await;
                Handled
            }
            _ => Super,
        }
    }

    /// Superstate for operational modes (solo and cooperative)
    ///
    /// Handles common operational logic:
    /// - Shutdown requests for graceful shutdown
    /// - StandbyShutdown requests for low power mode
    /// - ExternalPowerOff events that trigger blackout transitions
    ///
    /// Child states: OperationalSolo, OperationalCoOp
    #[allow(unused_variables)]
    #[superstate(superstate = "powered_on")]
    async fn operational(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Shutdown => Transition(State::manual_shutdown(Instant::now())), // Graceful shutdown from operational mode
            Event::StandbyShutdown => Transition(State::entering_standby(Instant::now())),
            _ => Super,
        }
    }

    /// System is fully operational in solo mode
    ///
    /// Operating mode:
    /// - Solo mode: No host watchdog, independent operation
    /// - System operates independently without requiring host cooperation
    ///
    /// Hardware state:
    /// - All systems powered and operational
    /// - LED pattern shows solo mode (yellow)
    /// - Host watchdog is disabled
    ///
    /// Transitions:
    /// - Tick (when VIN <= threshold) -> BlackoutSolo (external power lost, running on supercap)
    /// - SetWatchdogTimeout(>0) -> OperationalCoOp (enable cooperative mode)
    /// - StandbyShutdown -> EnteringStandby (low power mode request) [handled by superstate]
    #[allow(unused_variables)]
    #[state(superstate = "operational", entry_action = "enter_operational_solo")]
    async fn operational_solo(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power is still available
                if !is_vin_power_available().await {
                    Transition(State::blackout_solo(Instant::now()))
                } else {
                    Super
                }
            }
            Event::SetWatchdogTimeout(timeout) => {
                if *timeout > 0 {
                    context.host_watchdog_timeout_ms = *timeout;
                    context.host_watchdog_last_ping = Instant::now();
                    Transition(State::operational_co_op())
                } else {
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_operational_solo(context: &mut Context) {
        context.set_led_pattern(&State::operational_solo()).await;
        context.host_watchdog_timeout_ms = 0; // Disable watchdog
    }

    /// System is fully operational in cooperative mode
    ///
    /// Operating mode:
    /// - Cooperative mode: Host must send periodic pings
    /// - Host watchdog monitoring is active
    ///
    /// Hardware state:
    /// - All systems powered and operational
    /// - LED pattern shows cooperative mode (green)
    /// - Host watchdog timeout monitoring active
    ///
    /// Transitions:
    /// - Tick (when VIN <= threshold) -> BlackoutCoOp (external power lost, running on supercap)
    /// - Tick (watchdog timeout) -> HostUnresponsive (host stopped responding)
    /// - SetWatchdogTimeout(0) -> OperationalSolo (disable cooperative mode)
    /// - StandbyShutdown -> EnteringStandby (low power mode request) [handled by superstate]
    #[allow(unused_variables)]
    #[state(superstate = "operational", entry_action = "enter_operational_co_op")]
    async fn operational_co_op(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power is still available
                if !is_vin_power_available().await {
                    return Transition(State::blackout_co_op(Instant::now()));
                }

                if Instant::now().duration_since(context.host_watchdog_last_ping)
                    > Duration::from_millis(context.host_watchdog_timeout_ms as u64)
                {
                    return Transition(State::host_unresponsive(Instant::now()));
                }
                Super
            }
            Event::SetWatchdogTimeout(timeout) => {
                if *timeout == 0 {
                    context.host_watchdog_timeout_ms = 0;
                    Transition(State::operational_solo())
                } else {
                    context.host_watchdog_timeout_ms = *timeout;
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_operational_co_op(context: &mut Context) {
        context.set_led_pattern(&State::operational_co_op()).await;
    }

    /// Superstate for blackout modes (solo and cooperative)
    ///
    /// Child states: BlackoutSolo, BlackoutCoOp
    #[allow(unused_variables)]
    #[superstate(superstate = "powered_on")]
    async fn blackout(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            _ => Super,
        }
    }

    /// System running on supercapacitor power in solo mode
    ///
    /// Operating mode:
    /// - Solo mode: Automatic shutdown after configurable timeout (default 30s)
    /// - No host watchdog cooperation
    ///
    /// Hardware state:
    /// - Running on supercapacitor power only
    /// - LED shows solo depleting pattern (orange)
    /// - Limited runtime based on supercap charge and power consumption
    ///
    /// Transitions:
    /// - Tick (when VIN > threshold) -> OperationalSolo (external power restored)
    /// - Tick (timeout) -> BlackoutShutdown (automatic shutdown after timeout)
    #[allow(unused_variables)]
    #[state(superstate = "blackout", entry_action = "enter_blackout_solo")]
    async fn blackout_solo(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power has been restored
                if is_vin_power_available().await {
                    return Transition(State::operational_solo());
                }

                // Solo mode: trigger shutdown after timeout
                let solo_depleting_timeout_ms = get_solo_depleting_timeout_ms().await;
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(solo_depleting_timeout_ms as u64)
                {
                    context
                        .send_power_button_event(PowerButtonEvents::DoubleClick)
                        .await;
                    return Transition(State::blackout_shutdown(Instant::now()));
                }
                Super
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_blackout_solo(entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&State::blackout_solo(Instant::now())).await;
    }

    /// System running on supercapacitor power in cooperative mode
    ///
    /// Operating mode:
    /// - Cooperative mode: Waits for host to initiate shutdown
    /// - Host watchdog cooperation continues during blackout
    ///
    /// Hardware state:
    /// - Running on supercapacitor power only
    /// - LED shows cooperative depleting pattern (dark olive green)
    /// - Limited runtime based on supercap charge and power consumption
    ///
    /// Transitions:
    /// - Tick (when VIN > threshold) -> OperationalCoOp (external power restored)
    /// - Shutdown event -> BlackoutShutdown (host-initiated shutdown)
    #[allow(unused_variables)]
    #[state(superstate = "blackout", entry_action = "enter_blackout_co_op")]
    async fn blackout_co_op(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check if external power has been restored
                if is_vin_power_available().await {
                    return Transition(State::operational_co_op());
                }
                Super
            }
            Event::Shutdown => {
                Transition(State::blackout_shutdown(Instant::now()))
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_blackout_co_op(entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&State::blackout_co_op(Instant::now())).await;
    }

    /// Blackout shutdown sequence in progress during power loss scenarios
    ///
    /// Purpose:
    /// - Allows host system time to save data and shut down gracefully
    /// - Prevents data corruption from sudden power loss
    /// - Configurable timeout for shutdown completion
    ///
    /// Hardware state:
    /// - System still powered but shutdown initiated
    /// - LED shows shutdown pattern
    /// - Monitoring for CM5 to complete shutdown
    ///
    /// Transitions:
    /// - ComputeModuleOff -> PoweredDownBlackout (CM5 completed graceful shutdown)
    /// - Timeout -> PoweredDownBlackout (forced shutdown after timeout expires)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_blackout_shutdown")]
    async fn blackout_shutdown(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let shutdown_wait_duration_ms = get_shutdown_wait_duration_ms().await;
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(shutdown_wait_duration_ms as u64)
                {
                    Transition(State::powered_down_blackout(Instant::now())) // Blackout shutdown (timeout)
                } else {
                    Super
                }
            }
            Event::ComputeModuleOff => Transition(State::powered_down_blackout(Instant::now())), // Blackout shutdown (CM5 shut down gracefully)
            _ => Super,
        }
    }

    #[action]
    async fn enter_blackout_shutdown(context: &mut Context) {
        context.set_led_pattern(&State::blackout_shutdown(Instant::now())).await;
    }

    /// Manual shutdown sequence in progress during normal operation
    ///
    /// Purpose:
    /// - Allows host system time to save data and shut down gracefully
    /// - Prevents data corruption from user/host-initiated shutdown
    /// - Configurable timeout for shutdown completion
    ///
    /// Hardware state:
    /// - System still powered but shutdown initiated
    /// - LED shows shutdown pattern
    /// - Monitoring for CM5 to complete shutdown
    ///
    /// Transitions:
    /// - ComputeModuleOff -> PoweredDownManual (CM5 completed graceful shutdown)
    /// - Timeout -> PoweredDownManual (forced shutdown after timeout expires)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_manual_shutdown")]
    async fn manual_shutdown(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let shutdown_wait_duration_ms = get_shutdown_wait_duration_ms().await;
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(shutdown_wait_duration_ms as u64)
                {
                    Transition(State::powered_down_manual(Instant::now())) // Manual shutdown (timeout)
                } else {
                    Super
                }
            }
            Event::ComputeModuleOff => Transition(State::powered_down_manual(Instant::now())), // Manual shutdown (CM5 shut down gracefully)
            _ => Super,
        }
    }

    #[action]
    async fn enter_manual_shutdown(context: &mut Context) {
        context.set_led_pattern(&State::manual_shutdown(Instant::now())).await;
    }

    /// System is powered down after blackout shutdown
    ///
    /// Restart behavior: Always restarts automatically after timeout
    /// - Blackout shutdowns are considered critical power events
    /// - System must restart to restore service availability
    /// - Power button and VIN power changes trigger immediate restart
    ///
    /// Hardware state:
    /// - All power rails disabled
    /// - LED shows off pattern
    /// - Waiting for restart timeout or trigger events
    ///
    /// Transitions:
    /// - Auto-restart timeout -> System reset (always restarts for blackout scenarios)
    /// - PowerButtonPress -> System reset (manual restart)
    /// - VIN power change -> System reset (power cycling recovery)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_powered_down_blackout")]
    async fn powered_down_blackout(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(OFF_STATE_DURATION_MS as u64)
                {
                    SCB::sys_reset();
                } else {
                    Super
                }
            }
            Event::PowerButtonPress => {
                // Power button press always triggers restart
                info!("Power button press detected in powered down blackout state, restarting system");
                SCB::sys_reset();
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_powered_down_blackout(context: &mut Context) {
        context.outputs.power_off();
        usb_power_off().await;
        context.set_led_pattern(&State::powered_down_blackout(Instant::now())).await;
    }

    /// System is powered down after manual/command-based shutdown
    ///
    /// Restart behavior: Respects auto_restart configuration for timeout-based restart
    /// - Manual shutdowns honor user preference for automatic restart
    /// - Power button and VIN power changes override auto_restart setting
    /// - If auto_restart is false, system stays off until manual intervention
    ///
    /// Hardware state:
    /// - All power rails disabled
    /// - LED shows off pattern
    /// - Waiting for restart conditions based on configuration
    ///
    /// Transitions:
    /// - Auto-restart timeout -> System reset (if auto_restart enabled)
    /// - PowerButtonPress -> System reset (manual restart, ignores auto_restart)
    /// - VIN power change -> System reset (power cycling recovery, ignores auto_restart)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_powered_down_manual")]
    async fn powered_down_manual(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                // Check for VIN power state changes (power cycling recovery)
                if !is_vin_power_available().await {
                    // VIN has been cut - trigger restart for power cycling recovery
                    info!("VIN blackout detected in powered down manual state, restarting system");
                    SCB::sys_reset();
                }

                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(OFF_STATE_DURATION_MS as u64)
                {
                    // For command-based shutdowns, respect the auto_restart setting
                    let auto_restart = get_auto_restart().await;
                    if auto_restart {
                        SCB::sys_reset();
                    }
                    // If auto_restart is false, stay in off state indefinitely
                    Super
                } else {
                    Super
                }
            }
            Event::PowerButtonPress => {
                // Power button press always triggers restart, regardless of auto_restart setting
                info!("Power button press detected in powered down manual state, restarting system");
                SCB::sys_reset();
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_powered_down_manual(context: &mut Context) {
        context.outputs.power_off();
        usb_power_off().await;
        context.set_led_pattern(&State::powered_down_manual(Instant::now())).await;
    }

    /// Host watchdog timeout occurred - system is unresponsive
    ///
    /// Purpose:
    /// - Indicates host system has stopped responding to watchdog pings
    /// - Provides opportunity for host to recover before forced reboot
    /// - Shows alert pattern to indicate system health issue
    ///
    /// Hardware state:
    /// - System still running but host considered unresponsive
    /// - LED shows watchdog alert pattern (typically red/orange warning)
    /// - Timeout countdown to forced reboot
    ///
    /// Transitions:
    /// - WatchdogPing -> Operational(cooperative) (host recovered)
    /// - Timeout -> PoweredDown (forced reboot due to unresponsive host)
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_host_unresponsive")]
    async fn host_unresponsive(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(HOST_WATCHDOG_REBOOT_DURATION_MS as u64)
                {
                    Transition(State::powered_down_blackout(Instant::now())) // Blackout shutdown (watchdog timeout)
                } else {
                    Super
                }
            }
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Transition(State::operational_co_op()) // Return to co-op mode
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_host_unresponsive(context: &mut Context) {
        context.set_led_pattern(&State::host_unresponsive(Instant::now())).await;
    }

    /// Transitioning to low-power standby mode
    ///
    /// Purpose:
    /// - Intermediate state for graceful transition to standby
    /// - Allows system to save state and prepare for low power mode
    /// - Host system initiates its own shutdown sequence
    ///
    /// Hardware state:
    /// - System still fully powered
    /// - LED shows standby shutdown pattern
    /// - Waiting for CM5 to complete shutdown
    ///
    /// Transitions:
    /// - ComputeModuleOff -> Standby (CM5 powered down, enter low power mode)
    /// - Timeout -> Standby (forced transition after timeout expires)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_entering_standby")]
    async fn entering_standby(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                let shutdown_wait_duration_ms = get_shutdown_wait_duration_ms().await;
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(shutdown_wait_duration_ms as u64)
                {
                    Transition(State::standby()) // Force transition to standby after timeout
                } else {
                    Super
                }
            }
            Event::ComputeModuleOff => Transition(State::standby()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_entering_standby(entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&State::entering_standby(Instant::now())).await;
    }

    /// Low-power standby mode with CM5 powered down
    ///
    /// Purpose:
    /// - Minimal power consumption while maintaining system availability
    /// - CM5 powered down but can wake on internal events
    /// - Preserves system state for quick wake-up
    ///
    /// Hardware state:
    /// - CM5 powered down (3.3V rail inactive)
    /// - Core system remains powered for wake capability
    /// - LED shows standby pattern (minimal/dim indication)
    ///
    /// Transitions:
    /// - ComputeModuleOn -> Operational(solo) (wake from standby, return to normal operation)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_standby")]
    async fn standby(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::ComputeModuleOn => Transition(State::operational_solo()), // Start in solo mode
            _ => Super,
        }
    }

    #[action]
    async fn enter_standby(context: &mut Context) {
        context.set_led_pattern(&State::standby()).await;
    }
}

#[task]
pub async fn state_machine_task(smor: StateMachineOutputResources) {
    info!("Starting state machine task");

    // Initialize resources
    let outputs = Outputs::new(smor);

    let mut context = Context::new(
        outputs,
        &POWER_BUTTON_EVENT_CHANNEL,
        &LED_BLINKER_EVENT_CHANNEL,
        0, // Host watchdog is initially disabled
    );

    let mut state_machine = HalpiStateMachine::default().state_machine();

    match STATE_MACHINE_STATE.init(Mutex::<NoopRawMutex, _>::new(*state_machine.state())) {
        Ok(_) => info!("State machine initialized successfully"),
        Err(_) => error!("Failed to initialize state machine"),
    }

    let mut ticker = Ticker::every(Duration::from_millis(50));

    let receiver = STATE_MACHINE_EVENT_CHANNEL.receiver();

    info!("State machine task initialized");

    let mut prev_cm_on = false;

    loop {
        // Handle state machine transitions
        ticker.next().await;

        let mut events_to_process = Vec::new();

        while !receiver.is_empty() {
            // Check for events from the channel
            let event = receiver.receive().await;
            match event {
                StateMachineEvents::Shutdown => {
                    events_to_process.push(Event::Shutdown);
                }
                StateMachineEvents::SetHostWatchdogTimeout(timeout) => {
                    events_to_process.push(Event::SetWatchdogTimeout(timeout));
                }
                StateMachineEvents::HostWatchdogPing => {
                    events_to_process.push(Event::WatchdogPing);
                }
                StateMachineEvents::StandbyShutdown => {
                    events_to_process.push(Event::StandbyShutdown);
                }
                StateMachineEvents::Off => {
                    events_to_process.push(Event::Off);
                }
                StateMachineEvents::PowerButtonPress => {
                    events_to_process.push(Event::PowerButtonPress);
                }
            }
        }

        // Generate events based on current inputs (edge detection)
        let inputs = INPUTS.lock().await;

        // CM state edge detection
        let cm_on = inputs.cm_on;
        if cm_on != prev_cm_on {
            if cm_on {
                events_to_process.push(Event::ComputeModuleOn);
            } else {
                events_to_process.push(Event::ComputeModuleOff);
            }
            prev_cm_on = cm_on;
        }

        drop(inputs);

        // Vscap alarm detection
        let (_, vscap_alarm) = get_vscap_status().await;
        if vscap_alarm && !context.vscap_alarm_active {
            events_to_process.push(Event::SupercapOvervoltage);
        }

        // Add a regular tick event
        events_to_process.push(Event::Tick);


        for event in events_to_process {
            // Handle each event
            state_machine
                .handle_with_context(&event, &mut context)
                .await;
            // Record the current state
            record_state_machine_state(state_machine.state()).await;
        }
    }
}
