use crate::led_patterns::{get_state_pattern, get_vscap_alarm_pattern};
use crate::tasks::config_manager::{get_auto_restart, get_shutdown_wait_duration_ms, get_solo_depleting_timeout_ms};
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
/// - Hardware events use descriptive names (ExternalPowerOn/Off, ComputeModuleOn/Off)
/// - Timer events use "Tick" for regular intervals
/// - User/host actions use descriptive verbs (Shutdown, WatchdogPing)
/// - Alarm events use specific names for safety-critical conditions (SupercapOvervoltage)
#[derive(Clone, Copy, Debug)]
pub enum Event {
    /// Regular timer tick (50ms intervals) for timeout and periodic checks
    Tick,
    /// External power (VIN 5V input) became available
    ExternalPowerOn,
    /// External power (VIN 5V input) was removed
    ExternalPowerOff,
    /// Supercapacitor voltage reached minimum threshold for operation
    SupercapReady,
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
    pub dis_usb3: Output<'static>,
    pub dis_usb2: Output<'static>,
    pub dis_usb1: Output<'static>,
    pub dis_usb0: Output<'static>,
}

impl Outputs {
    fn new(resources: StateMachineOutputResources) -> Self {
        Outputs {
            ven: Output::new(resources.ven, Level::Low),
            pcie_sleep: Output::new(resources.pcie_sleep, Level::Low),
            dis_usb0: Output::new(resources.dis_usb0, Level::High),
            dis_usb1: Output::new(resources.dis_usb1, Level::High),
            dis_usb2: Output::new(resources.dis_usb2, Level::High),
            dis_usb3: Output::new(resources.dis_usb3, Level::High),
        }
    }

    fn power_on(&mut self) {
        self.ven.set_high();
        self.pcie_sleep.set_low();
        self.dis_usb0.set_low();
        self.dis_usb1.set_low();
        self.dis_usb2.set_low();
        self.dis_usb3.set_low();
    }

    fn power_off(&mut self) {
        self.ven.set_low();
        self.pcie_sleep.set_high();
        self.dis_usb0.set_high();
        self.dis_usb1.set_high();
        self.dis_usb2.set_high();
        self.dis_usb3.set_high();
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
        State::Operational { co_op_enabled: false, .. } => "OperationalSolo",
        State::Operational { co_op_enabled: true, .. } => "OperationalCoOp",
        State::Blackout { co_op_enabled: false, .. } => "BlackoutSolo",
        State::Blackout { co_op_enabled: true, .. } => "BlackoutCoOp",
        State::GracefulShutdown { .. } => "GracefulShutdown",
        State::PoweredDown { .. } => "PoweredDown",
        State::HostUnresponsive { .. } => "HostUnresponsive",
        State::EnteringStandby {} => "EnteringStandby",
        State::Standby {} => "Standby",
    }
}

pub fn state_as_u8(state: &State) -> u8 {
    match state {
        State::PowerOff {} => 0,
        State::OffCharging {} => 1,
        State::SystemStartup {} => 2,
        State::Operational { co_op_enabled: false, .. } => 3, // Solo
        State::Operational { co_op_enabled: true, .. } => 4,  // CoOp
        State::Blackout { co_op_enabled: false, .. } => 5, // Solo
        State::Blackout { co_op_enabled: true, .. } => 6,  // CoOp
        State::GracefulShutdown { .. } => 7,
        State::PoweredDown { .. } => 8,
        State::HostUnresponsive { .. } => 9,
        State::EnteringStandby {} => 10,
        State::Standby {} => 11,
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
/// PowerOff ──ExternalPowerOn──> OffCharging ──SupercapReady──> SystemStartup ──ComputeModuleOn──> [PoweredOn]
///    ^                              ^                              ^                                 │
///    │                              │                              │                                 │
///    └─────ExternalPowerOff─────────┴─────ExternalPowerOff─────────┴─────────────────────────────────┘
///
/// [PoweredOn] (superstate)
/// ├── Operational (solo/cooperative modes)
/// │   └── ExternalPowerOff ──> Blackout ──ExternalPowerOn──> Operational
/// │                               │
/// │                               └── Timeout/Shutdown ──> GracefulShutdown ──> PoweredDown
/// ├── HostUnresponsive (host watchdog timeout)
/// │   ├── WatchdogPing ──> Operational(cooperative)
/// │   └── Timeout ──> PoweredDown
/// └── EnteringStandby ──ComputeModuleOff──> Standby ──ComputeModuleOn──> Operational(solo)
///
/// PoweredDown ──[restart conditions]──> System Reset
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
    /// - ExternalPowerOn -> OffCharging (external power applied)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_power_off")]
    async fn power_off(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::ExternalPowerOn => Transition(State::off_charging()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_power_off(&mut self, context: &mut Context) {
        context.outputs.power_off();
    }

    /// System is off but supercapacitor is charging from external power
    ///
    /// Hardware state:
    /// - External power (VIN) available but system not yet powered
    /// - Supercapacitor charging up to operational voltage
    /// - LED shows charging pattern
    ///
    /// Transitions:
    /// - SupercapReady -> SystemStartup (supercap charged enough to boot)
    /// - ExternalPowerOff -> PowerOff (external power removed)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off_charging")]
    async fn off_charging(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::SupercapReady => Transition(State::system_startup()),
            Event::ExternalPowerOff => Transition(State::power_off()),
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
    /// - ExternalPowerOff -> PowerOff (power lost during boot)
    #[state(entry_action = "enter_system_startup")]
    async fn system_startup(event: &Event) -> Outcome<State> {
        match event {
            Event::ComputeModuleOn => Transition(State::operational(false)), // Start in solo mode
            Event::ExternalPowerOff => Transition(State::power_off()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_system_startup(context: &mut Context) {
        context.outputs.power_on();
        context.set_led_pattern(&State::system_startup()).await;
    }

    /// Superstate for all situations where the system is powered on and running
    ///
    /// This superstate handles common events for all powered states:
    /// - SupercapOvervoltage: Activates persistent red LED warning for overvoltage (>10.5V)
    /// - ComputeModuleOff: Normal shutdown when CM5 powers itself down
    /// - Off: Force immediate shutdown
    /// - WatchdogPing: Updates host watchdog timer
    ///
    /// Child states: Operational, Blackout, HostUnresponsive
    #[allow(unused_variables)]
    #[superstate]
    async fn powered_on(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::ComputeModuleOff => Transition(State::powered_down(Instant::now(), true)), // Normal shutdown - CM5 turned itself off
            Event::Off => Transition(State::powered_down(Instant::now(), true)), // Force immediate shutdown
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

    /// System is fully operational with optional host watchdog cooperation
    ///
    /// Operating modes:
    /// - Solo mode (co_op_enabled=false): No host watchdog, independent operation
    /// - Cooperative mode (co_op_enabled=true): Host must send periodic pings
    ///
    /// Hardware state:
    /// - All systems powered and operational
    /// - LED pattern indicates current mode (solo/cooperative)
    ///
    /// Transitions:
    /// - ExternalPowerOff -> Blackout (external power lost, running on supercap)
    /// - Watchdog timeout -> HostUnresponsive (cooperative mode only)
    /// - StandbyShutdown -> EnteringStandby (low power mode request)
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_operational")]
    async fn operational(co_op_enabled: &mut bool, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                if *co_op_enabled {
                    // If the host watchdog is enabled, check for timeout
                    if context.host_watchdog_timeout_ms == 0 {
                        warn!("Host watchdog is disabled, but we are in co-op mode.");
                        *co_op_enabled = false;
                        // Update LED pattern when switching from co-op to solo mode
                        context.set_led_pattern(&State::operational(false)).await;
                        return Super;
                    }
                    if Instant::now().duration_since(context.host_watchdog_last_ping)
                        > Duration::from_millis(context.host_watchdog_timeout_ms as u64)
                    {
                        return Transition(State::host_unresponsive(Instant::now()));
                    }
                }
                Super
            }
            Event::SetWatchdogTimeout(timeout) => {
                context.host_watchdog_timeout_ms = *timeout;
                let new_co_op_enabled = *timeout > 0;
                if *co_op_enabled != new_co_op_enabled {
                    *co_op_enabled = new_co_op_enabled;
                    // Update LED pattern when co-op mode changes
                    context.set_led_pattern(&State::operational(*co_op_enabled)).await;
                }
                Super
            }
            Event::StandbyShutdown => Transition(State::entering_standby()),
            Event::ExternalPowerOff => {
                Transition(State::blackout(*co_op_enabled, Instant::now()))
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_operational(co_op_enabled: &mut bool, context: &mut Context) {
        if *co_op_enabled {
            context.set_led_pattern(&State::operational(true)).await;
        } else {
            context.set_led_pattern(&State::operational(false)).await;
            context.host_watchdog_timeout_ms = 0; // Disable watchdog
        }
    }

    /// System running on supercapacitor power after external power loss
    ///
    /// Operating modes:
    /// - Solo mode: Automatic shutdown after configurable timeout (default 30s)
    /// - Cooperative mode: Waits for host to initiate graceful shutdown
    ///
    /// Hardware state:
    /// - Running on supercapacitor power only
    /// - LED shows depleting pattern (different for solo/cooperative)
    /// - Limited runtime based on supercap charge and power consumption
    ///
    /// Transitions:
    /// - ExternalPowerOn -> Operational (external power restored)
    /// - Timeout -> GracefulShutdown (solo mode only, triggers graceful shutdown)
    /// - Shutdown event -> GracefulShutdown (cooperative mode, host-initiated)
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_blackout")]
    async fn blackout(
        co_op_enabled: &mut bool,
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                if !*co_op_enabled {
                    // Solo mode: trigger shutdown after timeout
                    let solo_depleting_timeout_ms = get_solo_depleting_timeout_ms().await;
                    let now = Instant::now();
                    if now.duration_since(*entry_time)
                        > Duration::from_millis(solo_depleting_timeout_ms as u64)
                    {
                        context
                            .send_power_button_event(PowerButtonEvents::DoubleClick)
                            .await;
                        return Transition(State::graceful_shutdown(Instant::now()));
                    }
                }
                // CoOp mode: wait for host to initiate shutdown
                Super
            }
            Event::Shutdown => {
                if *co_op_enabled {
                    Transition(State::graceful_shutdown(Instant::now()))
                } else {
                    Super
                }
            }
            Event::ExternalPowerOn => {
                Transition(State::operational(*co_op_enabled))
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_blackout(co_op_enabled: &mut bool, entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&State::blackout(*co_op_enabled, Instant::now())).await;
    }

    /// Graceful shutdown sequence in progress during power loss scenarios
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
    /// - ComputeModuleOff -> PoweredDown (CM5 completed graceful shutdown)
    /// - Timeout -> PoweredDown (forced shutdown after timeout expires)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_graceful_shutdown")]
    async fn graceful_shutdown(
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
                    Transition(State::powered_down(Instant::now(), false)) // Power-loss shutdown (timeout)
                } else {
                    Super
                }
            }
            Event::ComputeModuleOff => Transition(State::powered_down(Instant::now(), false)), // Power-loss shutdown (CM5 shut down gracefully)
            _ => Super,
        }
    }

    #[action]
    async fn enter_graceful_shutdown(context: &mut Context) {
        context.set_led_pattern(&State::graceful_shutdown(Instant::now())).await;
    }

    /// System is powered down and determining restart behavior
    ///
    /// Restart behavior depends on shutdown type:
    /// - Intentional shutdown: Respects auto_restart config setting
    /// - Power-loss shutdown: Always restarts automatically
    /// - Power button press: Always triggers restart regardless of config
    /// - External power events: Always trigger restart for power management
    ///
    /// Hardware state:
    /// - All power rails disabled
    /// - LED shows off pattern
    /// - Waiting for restart conditions or timeout
    ///
    /// Transitions:
    /// - Auto-restart timeout -> System reset (if conditions met)
    /// - PowerButtonPress -> System reset (manual restart)
    /// - ExternalPowerOff -> System reset (power management restart)
    #[allow(unused_variables)]
    #[state(entry_action = "enter_powered_down")]
    async fn powered_down(
        entry_time: &mut Instant,
        intentional_shutdown: &mut bool,
        event: &Event,
        context: &mut Context
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(OFF_STATE_DURATION_MS as u64)
                {
                    if *intentional_shutdown {
                        // For intentional shutdowns, respect the auto_restart setting
                        let auto_restart = get_auto_restart().await;
                        if auto_restart {
                            SCB::sys_reset();
                        }
                        // If auto_restart is false, stay in off state indefinitely
                    } else {
                        // For power-loss shutdowns, always restart automatically
                        SCB::sys_reset();
                    }
                    Super
                } else {
                    Super
                }
            }
            Event::PowerButtonPress => {
                // Power button press always triggers restart, regardless of auto_restart setting
                info!("Power button press detected in off state, restarting system");
                SCB::sys_reset();
            }
            Event::ExternalPowerOff => {
                // VIN power loss always triggers restart, regardless of auto_restart setting
                info!("VIN power loss detected in off state, restarting system");
                SCB::sys_reset();
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_powered_down(context: &mut Context) {
        context.outputs.power_off();
        context.set_led_pattern(&State::powered_down(Instant::now(), false)).await;
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
                    Transition(State::powered_down(Instant::now(), false)) // Power-loss shutdown
                } else {
                    Super
                }
            }
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Transition(State::operational(true)) // Return to co-op mode
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
    #[allow(unused_variables)]
    #[state(entry_action = "enter_entering_standby")]
    async fn entering_standby(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::ComputeModuleOff => Transition(State::standby()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_entering_standby(context: &mut Context) {
        context.set_led_pattern(&State::entering_standby()).await;
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
            Event::ComputeModuleOn => Transition(State::operational(false)), // Start in solo mode
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

    let mut prev_vin_power = false;
    let mut prev_vscap_ready = false;
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

        // VIN power edge detection
        let vin_power = inputs.vin > DEFAULT_VIN_POWER_THRESHOLD;
        if vin_power != prev_vin_power {
            if vin_power {
                events_to_process.push(Event::ExternalPowerOn);
            } else {
                events_to_process.push(Event::ExternalPowerOff);
            }
            prev_vin_power = vin_power;
        }

        // Supercap voltage edge detection
        let vscap_ready = inputs.vscap > DEFAULT_VSCAP_POWER_ON_THRESHOLD;
        if vscap_ready != prev_vscap_ready {
            if vscap_ready {
                events_to_process.push(Event::SupercapReady);
            }
            prev_vscap_ready = vscap_ready;
        }

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

        // Vscap alarm detection
        let vscap_alarm = inputs.vscap > VSCAP_MAX_ALARM;
        if vscap_alarm && !context.vscap_alarm_active {
            events_to_process.push(Event::SupercapOvervoltage);
        }

        // Add a regular tick event
        events_to_process.push(Event::Tick);

        drop(inputs);

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
