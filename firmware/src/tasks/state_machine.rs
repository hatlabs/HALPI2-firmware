use crate::led_patterns::get_state_pattern;
use crate::tasks::config_manager::get_shutdown_wait_duration_ms;
use crate::tasks::led_blinker::{LED_BLINKER_EVENT_CHANNEL, LEDBlinkerEvents};
use crate::tasks::power_button::{POWER_BUTTON_EVENT_CHANNEL, PowerButtonEvents};
use alloc::vec::Vec;
use core::fmt::Debug;
use cortex_m::peripheral::SCB;
use defmt::*;
use embassy_executor::task;
use embassy_rp::gpio::{Level, Output};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel;
use embassy_time::{Duration, Instant, Ticker};
use statig::prelude::*;

use crate::config::*;
use crate::config_resources::StateMachineOutputResources;
use crate::tasks::gpio_input::INPUTS;

use super::led_blinker::LEDBlinkerChannelType;
use super::power_button::PowerButtonChannelType;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HalpiStates {
    OffNoVin,
    OffCharging,
    Booting,
    OnNoWatchdog,
    OnWithWatchdog,
    DepletingNoWatchdog,
    DepletingWithWatchdog,
    Shutdown,
    Off,
    WatchdogAlert,
    SleepShutdown,
    Sleep,
}

#[allow(dead_code)]
pub enum StateMachineEvents {
    Shutdown,
    SleepShutdown,
    Off,
    SetHostWatchdogTimeout(u16),
    HostWatchdogPing,
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
    SleepShutdown,
    Off,
    SetWatchdogTimeout(u16),
    WatchdogPing,
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

    async fn set_led_pattern(&self, state: &HalpiStates) {
        let _ = self
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_state_pattern(state)))
            .await;
    }

    async fn send_power_button_event(&self, event: PowerButtonEvents) {
        let _ = self.power_button_channel.send(event).await;
    }
}

#[derive(Debug, Default)]
pub struct HalpiStateMachine {}

#[state_machine(
    initial = "State::off_no_vin()",
    before_transition = "Self::before_transition",
    state(derive(Debug)),
    superstate(derive(Debug))
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
        context.set_led_pattern(&HalpiStates::OffCharging).await;
    }

    /// 5V rail is powered on and we're waiting for the CM5 3.3V rail to come up.
    #[state(entry_action = "enter_booting")]
    async fn booting(event: &Event) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::on_no_watchdog()),
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_booting(context: &mut Context) {
        context.outputs.power_on();
        context.set_led_pattern(&HalpiStates::Booting).await;
    }

    /// Superstate for all situations where the system is powered on and running.
    #[allow(unused_variables)]
    #[superstate]
    async fn on(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOff => Transition(State::off(Instant::now())),
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Handled
            }
            _ => Super,
        }
    }

    /// Powered on, but no watchdog enabled.
    #[allow(unused_variables)]
    #[state(superstate = "on", entry_action = "enter_on_no_watchdog")]
    async fn on_no_watchdog(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::SetWatchdogTimeout(timeout) => {
                context.host_watchdog_timeout_ms = *timeout;
                if *timeout > 0 {
                    Transition(State::on_with_watchdog())
                } else {
                    Super
                }
            }
            Event::SleepShutdown => Transition(State::sleep_shutdown()),
            Event::VinPowerOff => Transition(State::depleting_no_watchdog(Instant::now())),
            _ => Super,
        }
    }

    #[action]
    async fn enter_on_no_watchdog(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::OnNoWatchdog).await;
        context.host_watchdog_timeout_ms = 0; // Disable watchdog
    }

    /// Powered on with watchdog enabled.
    #[allow(unused_variables)]
    #[state(superstate = "on", entry_action = "enter_on_with_watchdog")]
    async fn on_with_watchdog(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                // If the host watchdog is enabled, we will send a ping to the watchdog task.
                if context.host_watchdog_timeout_ms == 0 {
                    warn!("Host watchdog is disabled, but we are in the on_with_watchdog state.");
                    // Transition to the no watchdog state if the timeout is 0.
                    return Transition(State::on_no_watchdog());
                }
                if Instant::now().duration_since(context.host_watchdog_last_ping)
                    > Duration::from_millis(context.host_watchdog_timeout_ms as u64)
                {
                    Transition(State::watchdog_alert(Instant::now()))
                } else {
                    Super
                }
            }
            Event::SetWatchdogTimeout(timeout) => {
                context.host_watchdog_timeout_ms = *timeout;
                if *timeout == 0 {
                    Transition(State::on_no_watchdog())
                } else {
                    Transition(State::on_with_watchdog())
                }
            }
            Event::SleepShutdown => Transition(State::sleep_shutdown()),
            Event::VinPowerOff => Transition(State::depleting_with_watchdog()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_on_with_watchdog(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::OnWithWatchdog).await;
    }

    /// If the host watchdog is not enabled, we will trigger shutdown after a timeout.
    #[allow(unused_variables)]
    #[state(superstate = "on", entry_action = "enter_depleting_no_watchdog")]
    async fn depleting_no_watchdog(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(DEFAULT_DEPLETING_TIMEOUT_MS as u64)
                {
                    context
                        .send_power_button_event(PowerButtonEvents::DoubleClick)
                        .await;
                    Transition(State::shutdown(Instant::now()))
                } else {
                    Super
                }
            }
            Event::VinPowerOn => Transition(State::on_no_watchdog()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_depleting_no_watchdog(entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context
            .set_led_pattern(&HalpiStates::DepletingNoWatchdog)
            .await;
    }

    /// If the host watchdog is enabled, we will wait for the host to initiate shutdown
    #[allow(unused_variables)]
    #[state(superstate = "on", entry_action = "enter_depleting_with_watchdog")]
    async fn depleting_with_watchdog(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Shutdown => Transition(State::shutdown(Instant::now())),
            Event::VinPowerOn => Transition(State::on_with_watchdog()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_depleting_with_watchdog(context: &mut Context) {
        context
            .set_led_pattern(&HalpiStates::DepletingWithWatchdog)
            .await;
    }

    /// Shutdown state, where the system is waiting for the shutdown to complete.
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
                    Transition(State::off(Instant::now()))
                } else {
                    Super
                }
            }
            Event::CmOff => Transition(State::off(Instant::now())),
            _ => Super,
        }
    }

    #[action]
    async fn enter_shutdown(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::Shutdown).await;
    }

    /// Turned off. Will reboot after 5 seconds if no VIN power is detected.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_off")]
    async fn off(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
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
            _ => Super,
        }
    }

    #[action]
    async fn enter_off(context: &mut Context) {
        context.outputs.power_off();
        context.set_led_pattern(&HalpiStates::Off).await;
    }

    // This state is triggered if the host watchdog is enabled and a watchdog timeout occurs.
    #[allow(unused_variables)]
    #[state(superstate = "on", entry_action = "enter_watchdog_alert")]
    async fn watchdog_alert(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_secs(HOST_WATCHDOG_REBOOT_DURATION_MS as u64)
                {
                    Transition(State::off(Instant::now()))
                } else {
                    Super
                }
            }
            Event::WatchdogPing => {
                context.host_watchdog_last_ping = Instant::now();
                Transition(State::on_with_watchdog())
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_watchdog_alert(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::WatchdogAlert).await;
    }

    /// Shutdown to a sleep state, where the system is waiting for the CM to power on.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_sleep_shutdown")]
    async fn sleep_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::CmOff => Transition(State::sleep()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_sleep_shutdown(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::SleepShutdown).await;
    }

    /// Sleep state. The CM5 is shut down but may wake up on internal events.
    #[allow(unused_variables)]
    #[state(entry_action = "enter_sleep")]
    async fn sleep(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            // FIXME: Which events should be handled here?
            Event::CmOn => Transition(State::on_no_watchdog()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_sleep(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::Sleep).await;
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

    let mut ticker = Ticker::every(Duration::from_millis(50));

    let receiver = STATE_MACHINE_EVENT_CHANNEL.receiver();

    info!("State machine task initialized");

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
                StateMachineEvents::SleepShutdown => {
                    events_to_process.push(Event::SleepShutdown);
                }
                StateMachineEvents::Off => {
                    events_to_process.push(Event::Off);
                }
            }
        }

        // Generate events based on current inputs
        let inputs = INPUTS.lock().await;

        // Check VIN power
        if inputs.vin > DEFAULT_VIN_POWER_THRESHOLD {
            events_to_process.push(Event::VinPowerOn);
        } else {
            events_to_process.push(Event::VinPowerOff);
        }

        // Check supercap voltage
        if inputs.vscap > DEFAULT_VSCAP_POWER_ON_THRESHOLD {
            events_to_process.push(Event::VscapReady);
        }

        // Check CM state
        if inputs.cm_on {
            events_to_process.push(Event::CmOn);
        } else {
            events_to_process.push(Event::CmOff);
        }

        // Add a regular tick event
        events_to_process.push(Event::Tick);

        drop(inputs);

        for event in events_to_process {
            // Handle each event
            state_machine
                .handle_with_context(&event, &mut context)
                .await;
        }
    }
}
