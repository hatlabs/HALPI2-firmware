use crate::led_patterns::get_state_pattern;
use crate::tasks::config_manager::get_shutdown_wait_duration_ms;
use crate::tasks::led_blinker::{LED_BLINKER_EVENT_CHANNEL, LEDBlinkerEvents};
use crate::tasks::power_button::{POWER_BUTTON_EVENT_CHANNEL, PowerButtonEvents};
use core::fmt::Debug;
use alloc::vec::Vec;
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TargetState {
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

pub enum StateMachineEvents {
    TriggerShutdown,
    TriggerSleepShutdown,
    TriggerOff,
    TriggerWatchdogReboot,
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
struct Outputs {
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
    pub last_state_entry: Instant,
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
            last_state_entry: Instant::now(),
        }
    }

    async fn set_led_pattern(&self, state: &TargetState) {
        let _ = self
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_state_pattern(state)))
            .await;
    }

    async fn send_power_button_event(&self, event: PowerButtonEvents) {
        let _ = self.power_button_channel.send(event).await;
    }

    fn time_since_entry(&self) -> Duration {
        Instant::now().duration_since(self.last_state_entry)
    }

    fn update_entry_time(&mut self) {
        self.last_state_entry = Instant::now();
    }
}

#[derive(Debug, Default)]
pub struct HalpiStateMachine {}

#[state_machine(
    initial = "State::init()",
    state(derive(Debug)),
    superstate(derive(Debug))
)]
impl HalpiStateMachine {

    #[state()]
    async fn init(event: &Event) -> Outcome<State> {
        Transition(State::off_no_vin())
    }

    #[state(, entry_action = "enter_off_no_vin")]
    async fn off_no_vin(event: &Event) -> Outcome<State> {
        match event {
            Event::VinPowerOn => Transition(State::off_charging()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_off_no_vin(_event: &Event, context: &mut Context) -> Outcome<State> {
        context.outputs.power_off();
        Handled
    }

    #[state(, entry_action = "enter_off_charging")]
    async fn off_charging(event: &Event) -> Outcome<State> {
        match event {
            Event::VscapReady => Transition(State::booting()),
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_off_charging(_event: &Event, context: &mut Context) -> Outcome<State> {
        context.set_led_pattern(&TargetState::OffCharging).await;
        Handled
    }

    #[state(, entry_action = "enter_booting")]
    async fn booting(event: &Event) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::on()),
            Event::VinPowerOff => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_booting(_event: &Event, context: &mut Context) -> Outcome<State> {
        context.outputs.power_on();
        context.set_led_pattern(&TargetState::Booting).await;
        Handled
    }

    #[state(, entry_action = "enter_on")]
    async fn on(event: &Event) -> Outcome<State> {
        match event {
            Event::VinPowerOff => Transition(State::depleting(Instant::now())),
            Event::CmOff => {
                SCB::sys_reset();
                Transition(State::off_no_vin())
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_on(event: &Event, context: &mut Context) -> Outcome<State> {
        context.set_led_pattern(&TargetState::On).await;
        Handled
    }

    #[state(, entry_action = "enter_depleting")]
    async fn depleting(
        entry_time: &mut Instant,
        event: &Event,
    ) -> Outcome<State> {
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
    async fn enter_depleting(
        entry_time: &mut Instant,
        _event: &Event,
        context: &mut Context,
    ) -> Outcome<State> {
        *entry_time = Instant::now();
        context.set_led_pattern(&TargetState::Depleting).await;
        context.update_entry_time();
        Handled
    }

    #[state(, entry_action = "enter_shutdown")]
    async fn shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::Tick => {
                let shutdown_wait_duration_ms = get_shutdown_wait_duration_ms().await;
                let now = Instant::now();
                if now.duration_since(context.last_state_entry)
                    > Duration::from_millis(shutdown_wait_duration_ms as u64)
                {
                    Transition(State::off(Instant::now()))
                } else {
                    Super
                }
            },
            Event::CmOn => Transition(State::off(Instant::now())),
            _ => Super,
        }
    }

    #[action]
    async fn enter_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        context
            .send_power_button_event(PowerButtonEvents::DoubleClick)
            .await;
        context.set_led_pattern(&TargetState::Shutdown).await;
        context.update_entry_time();
        Handled
    }

    #[state(, entry_action = "enter_off")]
    async fn off(entry_time: &mut Instant, event: &Event) -> Outcome<State> {
        match event {
            Event::VinPowerOn => Transition(State::off_charging()),
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time) > Duration::from_secs(5) {
                    SCB::sys_reset();
                }
                Super
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_off(_entry_time: &mut Instant, _event: &Event, context: &mut Context) -> Outcome<State> {
        context.outputs.power_off();
        context.set_led_pattern(&TargetState::Off).await;
        context.update_entry_time();
        Handled
    }

    #[state(
        ,
        entry_action = "enter_watchdog_reboot"
    )]
    async fn watchdog_reboot(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::off(Instant::now())),
            Event::Tick => {
                let now = Instant::now();
                if now.duration_since(*entry_time) > Duration::from_secs(5) {
                    Transition(State::off(Instant::now()))
                } else {
                    Super
                }
            }
            _ => Super,
        }
    }

    #[action]
    async fn enter_watchdog_reboot(entry_time: &mut Instant, event: &Event, context: &mut Context) -> Outcome<State> {
        context.set_led_pattern(&TargetState::WatchdogReboot).await;
        context.update_entry_time();
        Handled
    }

    #[state(, entry_action = "enter_sleep_shutdown")]
    async fn sleep_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::sleep()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_sleep_shutdown(event: &Event, context: &mut Context) -> Outcome<State> {
        context.set_led_pattern(&TargetState::SleepShutdown).await;
        Handled
    }

    #[state(, entry_action = "enter_sleep")]
    async fn sleep(event: &Event) -> Outcome<State> {
        match event {
            Event::CmOn => Transition(State::off_no_vin()),
            _ => Super,
        }
    }

    #[action]
    async fn enter_sleep(event: &Event, context: &mut Context) -> Outcome<State> {
        context.set_led_pattern(&TargetState::Sleep).await;
        Handled
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
                StateMachineEvents::TriggerShutdown => {
                    state_machine.handle_with_context(&mut Event::Shutdown, &mut context);
                },
                StateMachineEvents::TriggerWatchdogReboot => {
                    state_machine.handle_with_context(&mut Event::WatchdogReboot, &mut context);
                },
                StateMachineEvents::TriggerSleepShutdown => {
                    state_machine.handle_with_context(&mut Event::SleepShutdown, &mut context);
                },
                StateMachineEvents::TriggerOff => {
                    state_machine.handle_with_context(&mut Event::Off, &mut context);
                },
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
    }
}
