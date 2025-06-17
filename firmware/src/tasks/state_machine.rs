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
    On,
    Depleting,
    Shutdown,
    Off,
    WatchdogReboot,
    SleepShutdown,
    Sleep,
}

#[allow(dead_code)]
pub enum StateMachineEvents {
    Shutdown,
    SleepShutdown,
    Off,
    WatchdogReboot,
}

pub type StateMachineChannelType = channel::Channel<CriticalSectionRawMutex, StateMachineEvents, 8>;
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
    WatchdogReboot,
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
}

impl Context {
    pub fn new(
        outputs: Outputs,
        power_button_channel: &'static PowerButtonChannelType,
        led_blinker_channel: &'static LEDBlinkerChannelType,
    ) -> Self {
        Context {
            outputs,
            power_button_channel,
            led_blinker_channel,
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
        info!("Transitioning from {:?} to {:?}", defmt::Debug2Format(source), defmt::Debug2Format(target));
    }

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

    #[state(entry_action = "enter_booting")]
    async fn booting(event: &Event) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::on()),
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_booting(context: &mut Context) {
        context.outputs.power_on();
        context.set_led_pattern(&HalpiStates::Booting).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_on")]
    async fn on(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::VinPowerOff => Transition(State::depleting(Instant::now())),
            Event::CmOff => {
                SCB::sys_reset();
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_on(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::On).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_depleting")]
    async fn depleting(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time)
                    > Duration::from_millis(DEFAULT_DEPLETING_TIMEOUT_MS as u64)
                {
                    Transition(State::off_no_vin())
                } else {
                    Super
                }
            }
            Event::VinPowerOn => Transition(State::on()),
            Event::CmOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_depleting(entry_time: &mut Instant, context: &mut Context) {
        *entry_time = Instant::now();
        context.set_led_pattern(&HalpiStates::Depleting).await;
        *entry_time = Instant::now();
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_shutdown")]
    async fn shutdown(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
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
            Event::CmOn => Transition(State::off(Instant::now())),
            _ => Super,
        }
    }

    #[action]
    async fn enter_shutdown(context: &mut Context) {
        context
            .send_power_button_event(PowerButtonEvents::DoubleClick)
            .await;
        context.set_led_pattern(&HalpiStates::Shutdown).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_off")]
    async fn off(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::VinPowerOn => Transition(State::off_charging()),
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time) > Duration::from_secs(5) {
                    SCB::sys_reset();
                }
                Handled
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_off(context: &mut Context) {
        context.outputs.power_off();
        context.set_led_pattern(&HalpiStates::Off).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_watchdog_reboot")]
    async fn watchdog_reboot(
        entry_time: &mut Instant,
        event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::off(Instant::now())),
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
            _ => Super,
        }
    }

    #[action]
    async fn enter_watchdog_reboot(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::WatchdogReboot).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_sleep_shutdown")]
    async fn sleep_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::sleep()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_sleep_shutdown(context: &mut Context) {
        context.set_led_pattern(&HalpiStates::SleepShutdown).await;
    }

    #[allow(unused_variables)]
    #[state(entry_action = "enter_sleep")]
    async fn sleep(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::off_no_vin()),
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
    );

    let mut state_machine = HalpiStateMachine::default().state_machine();

    let mut ticker = Ticker::every(Duration::from_millis(500));

    let receiver = STATE_MACHINE_EVENT_CHANNEL.receiver();

    info!("State machine task initialized");

    loop {
        // Handle state machine transitions
        ticker.next().await;

        if !receiver.is_empty() {
            // Check for events from the channel
            let event = receiver.receive().await;
            match event {
                StateMachineEvents::Shutdown => {
                    let _ = state_machine
                        .handle_with_context(&Event::Shutdown, &mut context)
                        .await;
                }
                StateMachineEvents::WatchdogReboot => {
                    let _ = state_machine
                        .handle_with_context(&Event::WatchdogReboot, &mut context)
                        .await;
                }
                StateMachineEvents::SleepShutdown => {
                    let _ = state_machine
                        .handle_with_context(&Event::SleepShutdown, &mut context)
                        .await;
                }
                StateMachineEvents::Off => {
                    let _ = state_machine
                        .handle_with_context(&Event::Off, &mut context)
                        .await;
                }
            }
        }

        // Generate events based on current inputs
        let inputs = INPUTS.lock().await;
        let mut events_to_process = Vec::new();

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
