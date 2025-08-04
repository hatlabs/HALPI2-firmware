use crate::led_patterns::get_state_pattern;
use crate::tasks::config_manager::{get_auto_restart, get_shutdown_wait_duration_ms};
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


#[allow(dead_code)]
pub enum StateMachineEvents {
    Shutdown,
    StandbyShutdown,
    Off,
    SetHostWatchdogTimeout(u16),
    HostWatchdogPing,
    PowerButtonPress,
}

pub type StateMachineChannelType =
    channel::Channel<CriticalSectionRawMutex, StateMachineEvents, 16>;
pub static STATE_MACHINE_EVENT_CHANNEL: StateMachineChannelType = channel::Channel::new();

// Events used by the state machine
#[derive(Clone, Copy, Debug)]
pub enum Event {
    Tick,
    VinPowerOn,
    VinPowerOff,
    VscapReady,
    CmOn,
    CmOff,
    Shutdown,
    StandbyShutdown,
    Off,
    SetWatchdogTimeout(u16),
    WatchdogPing,
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
        }
    }

    async fn set_led_pattern(&self, state: &State) {
        let _ = self
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_state_pattern(state)))
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
        State::OffNoVin {} => "OffNoVin",
        State::OffCharging {} => "OffCharging",
        State::Booting {} => "Booting",
        State::On { co_op_enabled: false, .. } => "OnSolo",
        State::On { co_op_enabled: true, .. } => "OnCoOp",
        State::Depleting { co_op_enabled: false, .. } => "DepletingSolo",
        State::Depleting { co_op_enabled: true, .. } => "DepletingCoOp",
        State::Shutdown { .. } => "Shutdown",
        State::Off { .. } => "Off",
        State::WatchdogAlert { .. } => "WatchdogAlert",
        State::StandbyShutdown {} => "StandbyShutdown",
        State::Standby {} => "Standby",
    }
}

pub fn state_as_u8(state: &State) -> u8 {
    match state {
        State::OffNoVin {} => 0,
        State::OffCharging {} => 1,
        State::Booting {} => 2,
        State::On { co_op_enabled: false, .. } => 3, // Solo
        State::On { co_op_enabled: true, .. } => 4,  // CoOp
        State::Depleting { co_op_enabled: false, .. } => 5, // Solo
        State::Depleting { co_op_enabled: true, .. } => 6,  // CoOp
        State::Shutdown { .. } => 7,
        State::Off { .. } => 8,
        State::WatchdogAlert { .. } => 9,
        State::StandbyShutdown {} => 10,
        State::Standby {} => 11,
    }
}

pub async fn record_state_machine_state(state: &State) {
    *STATE_MACHINE_STATE.get().await.lock().await = *state;
}

#[derive(Debug, Default)]
pub struct HalpiStateMachine {}

#[state_machine(
    initial = "State::off_no_vin()",
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

    /// Turned off and no voltage on VIN.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off_no_vin")]
    async fn off_no_vin(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::VinPowerOn => Transition(State::off_charging()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_off_no_vin(&mut self, context: &mut Context) {
        context.outputs.power_off();
    }

    /// Turned off, but supercap is charging.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off_charging")]
    async fn off_charging(&mut self, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::VscapReady => Transition(State::booting()),
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_off_charging(&mut self, context: &mut Context) {
        context.set_led_pattern(&State::off_charging()).await;
    }

    /// 5V rail is powered on and we're waiting for the CM5 3.3V rail to come up.
    #[state(entry_action = "enter_booting")]
    async fn booting(event: &Event) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::on(false)), // Start in solo mode
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_booting(context: &mut Context) {
        context.outputs.power_on();
        context.set_led_pattern(&State::booting()).await;
    }

    /// Superstate for all situations where the system is powered on and running.
    #[allow(unused_variables)]
    #[superstate]
    async fn powered_on(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOff => Transition(State::off(Instant::now(), true)), // Normal shutdown - CM5 turned itself off
            Event::Off => Transition(State::off(Instant::now(), true)), // Force immediate shutdown
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Handled
            }
            _ => Super,
        }
    }

    /// Powered on, with optional watchdog cooperation.
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_on")]
    async fn on(co_op_enabled: &mut bool, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                if *co_op_enabled {
                    // If the host watchdog is enabled, check for timeout
                    if context.host_watchdog_timeout_ms == 0 {
                        warn!("Host watchdog is disabled, but we are in co-op mode.");
                        *co_op_enabled = false;
                        // Update LED pattern when switching from co-op to solo mode
                        context.set_led_pattern(&State::on(false)).await;
                        return Super;
                    }
                    if Instant::now().duration_since(context.host_watchdog_last_ping)
                        > Duration::from_millis(context.host_watchdog_timeout_ms as u64)
                    {
                        return Transition(State::watchdog_alert(Instant::now()));
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
                    context.set_led_pattern(&State::on(*co_op_enabled)).await;
                }
                Super
            }
            Event::StandbyShutdown => Transition(State::standby_shutdown()),
            Event::VinPowerOff => {
                Transition(State::depleting(*co_op_enabled, Instant::now()))
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_on(co_op_enabled: &mut bool, context: &mut Context) {
        if *co_op_enabled {
            context.set_led_pattern(&State::on(true)).await;
        } else {
            context.set_led_pattern(&State::on(false)).await;
            context.host_watchdog_timeout_ms = 0; // Disable watchdog
        }
    }

    /// Power is depleting, behavior depends on watchdog cooperation mode.
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_depleting")]
    async fn depleting(
        co_op_enabled: &mut bool,
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                if !*co_op_enabled {
                    // Solo mode: trigger shutdown after timeout
                    let now = Instant::now();
                    if now.duration_since(*entry_time)
                        > Duration::from_millis(DEFAULT_DEPLETING_TIMEOUT_MS as u64)
                    {
                        context
                            .send_power_button_event(PowerButtonEvents::DoubleClick)
                            .await;
                        return Transition(State::shutdown(Instant::now()));
                    }
                }
                // CoOp mode: wait for host to initiate shutdown
                Super
            }
            Event::Shutdown => {
                if *co_op_enabled {
                    Transition(State::shutdown(Instant::now()))
                } else {
                    Super
                }
            }
            Event::VinPowerOn => {
                Transition(State::on(*co_op_enabled))
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_depleting(co_op_enabled: &mut bool, entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&State::depleting(*co_op_enabled, Instant::now())).await;
    }

    /// Shutdown state, where the system is waiting for the shutdown to complete.
    /// This state is only reached during power-loss scenarios.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_shutdown")]
    async fn shutdown(
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
                    Transition(State::off(Instant::now(), false)) // Power-loss shutdown (timeout)
                } else {
                    Super
                }
            }
            Event::CmOff => Transition(State::off(Instant::now(), false)), // Power-loss shutdown (CM5 shut down gracefully)
            _ => Super,
        }
    }

    #[action]
    async fn enter_shutdown(context: &mut Context) {
        context.set_led_pattern(&State::shutdown(Instant::now())).await;
    }

    /// Turned off. Will reboot after 5 seconds if no VIN power is detected.
    /// For intentional shutdowns, respects auto_restart setting.
    /// For power-loss shutdowns, always restarts automatically.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off")]
    async fn off(
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
            Event::VinPowerOff => {
                // VIN power loss always triggers restart, regardless of auto_restart setting
                info!("VIN power loss detected in off state, restarting system");
                SCB::sys_reset();
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_off(context: &mut Context) {
        context.outputs.power_off();
        context.set_led_pattern(&State::off(Instant::now(), false)).await;
    }

    // This state is triggered if the host watchdog is enabled and a watchdog timeout occurs.
    #[allow(unused_variables)]
    #[state(superstate = "powered_on", entry_action = "enter_watchdog_alert")]
    async fn watchdog_alert(
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
                    Transition(State::off(Instant::now(), false)) // Power-loss shutdown
                } else {
                    Super
                }
            }
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Transition(State::on(true)) // Return to co-op mode
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_watchdog_alert(context: &mut Context) {
        context.set_led_pattern(&State::watchdog_alert(Instant::now())).await;
    }

    /// Shutdown to a standby state, where the system is waiting for the CM to power on.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_standby_shutdown")]
    async fn standby_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::CmOff => Transition(State::standby()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_standby_shutdown(context: &mut Context) {
        context.set_led_pattern(&State::standby_shutdown()).await;
    }

    /// Standby state. The CM5 is shut down but may wake up on internal events.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_standby")]
    async fn standby(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::CmOn => Transition(State::on(false)), // Start in solo mode
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
                events_to_process.push(Event::VinPowerOn);
            } else {
                events_to_process.push(Event::VinPowerOff);
            }
            prev_vin_power = vin_power;
        }

        // Supercap voltage edge detection
        let vscap_ready = inputs.vscap > DEFAULT_VSCAP_POWER_ON_THRESHOLD;
        if vscap_ready != prev_vscap_ready {
            if vscap_ready {
                events_to_process.push(Event::VscapReady);
            }
            prev_vscap_ready = vscap_ready;
        }

        // CM state edge detection
        let cm_on = inputs.cm_on;
        if cm_on != prev_cm_on {
            if cm_on {
                events_to_process.push(Event::CmOn);
            } else {
                events_to_process.push(Event::CmOff);
            }
            prev_cm_on = cm_on;
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
