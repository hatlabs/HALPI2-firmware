use crate::led_patterns::get_state_pattern;
use crate::tasks::led_blinker::LEDBlinkerEvents;
use crate::tasks::power_button::PowerButtonEvents;
use core::fmt::Debug;
use defmt::*;
use embassy_executor::task;
use embassy_rp::gpio::{Level, Output};
use embassy_time::{Duration, Instant, Ticker};

use crate::config_resources::StateMachineOutputResources;
use crate::tasks::gpio_input::INPUTS;
use crate::{LEDBlinkerChannelType, PowerButtonChannelType, config::*};


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
}

struct StateMachineContext {
    pub outputs: Outputs,
    pub power_button_channel: &'static PowerButtonChannelType,
    pub led_blinker_channel: &'static LEDBlinkerChannelType,
}

impl StateMachineContext {
    pub fn new(
        outputs: Outputs,
        power_button_channel: &'static PowerButtonChannelType,
        led_blinker_channel: &'static LEDBlinkerChannelType,
    ) -> Self {
        StateMachineContext {
            outputs,
            power_button_channel,
            led_blinker_channel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InitState {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffNoVinState {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffChargingState {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BootingState {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OnState {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepletingState {
    entry_time: Instant,
}

impl DepletingState {
    pub fn new() -> Self {
        DepletingState {
            entry_time: Instant::now(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShutdownState {
    entry_time: Instant,
}

impl ShutdownState {
    pub fn new() -> Self {
        ShutdownState {
            entry_time: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffState {}
impl OffState {
    pub fn new() -> Self {
        OffState {}
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum StateMachine {
    Init(InitState),
    OffNoVin(OffNoVinState),
    OffCharging(OffChargingState),
    Booting(BootingState),
    On(OnState),
    Depleting(DepletingState),
    Shutdown(ShutdownState),
    Off(OffState),
}

impl StateMachine {
    fn new() -> Self {
        StateMachine::Init(InitState {})
    }
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        match self {
            StateMachine::Init(state) => state.enter(context).await,
            StateMachine::OffNoVin(state) => state.enter(context).await,
            StateMachine::OffCharging(state) => state.enter(context).await,
            StateMachine::Booting(state) => state.enter(context).await,
            StateMachine::On(state) => state.enter(context).await,
            StateMachine::Depleting(state) => state.enter(context).await,
            StateMachine::Shutdown(state) => state.enter(context).await,
            StateMachine::Off(state) => state.enter(context).await,
        }
    }
    async fn exit(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        match self {
            StateMachine::Init(state) => state.exit(context).await,
            StateMachine::OffNoVin(state) => state.exit(context).await,
            StateMachine::OffCharging(state) => state.exit(context).await,
            StateMachine::Booting(state) => state.exit(context).await,
            StateMachine::On(state) => state.exit(context).await,
            StateMachine::Depleting(state) => state.exit(context).await,
            StateMachine::Shutdown(state) => state.exit(context).await,
            StateMachine::Off(state) => state.exit(context).await,
        }
    }
    async fn run(&mut self, context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        match self {
            StateMachine::Init(state) => state.run(context).await,
            StateMachine::OffNoVin(state) => state.run(context).await,
            StateMachine::OffCharging(state) => state.run(context).await,
            StateMachine::Booting(state) => state.run(context).await,
            StateMachine::On(state) => state.run(context).await,
            StateMachine::Depleting(state) => state.run(context).await,
            StateMachine::Shutdown(state) => state.run(context).await,
            StateMachine::Off(state) => state.run(context).await,
        }
    }
}

#[allow(dead_code)]
trait State
where
    Self: Sized,
{
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()>;
    async fn run(&mut self, context: &mut StateMachineContext) -> Result<StateMachine, ()>;
    async fn exit(&mut self, context: &mut StateMachineContext) -> Result<(), ()>;
    async fn set_led_pattern(
        &mut self,
        context: &StateMachineContext,
        state: &StateMachine,
    ) -> Result<(), ()> {
        context
            .led_blinker_channel
            .send(LEDBlinkerEvents::SetPattern(get_state_pattern(state)))
            .await;
        Ok(())
    }
}

impl State for InitState {
    async fn enter(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering InitState");
        // Initialize state
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        // Propagate to next state immediately
        Ok(StateMachine::OffNoVin(OffNoVinState {}))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting InitState");
        // Cleanup state
        Ok(())
    }
}

impl State for OffNoVinState {
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering OffNoVinState");
        // Initialize state
        // Set Ven low
        context.outputs.ven.set_low();
        context.outputs.pcie_sleep.set_high();
        context.outputs.dis_usb0.set_high();
        context.outputs.dis_usb1.set_high();
        context.outputs.dis_usb2.set_high();
        context.outputs.dis_usb3.set_high();
        // Set the LED blink pattern
        self.set_led_pattern(context, &StateMachine::OffNoVin(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        if inputs.vin > VIN_OFF {
            // Transition to OffChargingState
            return Ok(StateMachine::OffCharging(OffChargingState {}));
        }
        Ok(StateMachine::OffNoVin(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting OffNoVinState");
        // Cleanup state
        Ok(())
    }
}

impl State for OffChargingState {
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering OffChargingState");
        // Set the LED blink pattern
        self.set_led_pattern(context, &StateMachine::OffCharging(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        if inputs.vscap > VSCAP_POWER_ON {
            // Transition to BootingState
            return Ok(StateMachine::Booting(BootingState {}));
        }
        if inputs.vin < VIN_OFF {
            // Transition to OffNoVinState
            return Ok(StateMachine::OffNoVin(OffNoVinState {}));
        }
        Ok(StateMachine::OffCharging(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting OffChargingState");
        // Cleanup state
        Ok(())
    }
}

impl State for BootingState {
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering BootingState");
        // Enable the 5V output
        context.outputs.ven.set_high();
        context.outputs.pcie_sleep.set_low();
        context.outputs.dis_usb0.set_low();
        context.outputs.dis_usb1.set_low();
        context.outputs.dis_usb2.set_low();
        context.outputs.dis_usb3.set_low();
        // Set the LED blink pattern
        self.set_led_pattern(context, &StateMachine::Booting(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        if inputs.cm_on {
            // Transition to OnState
            return Ok(StateMachine::On(OnState {}));
        }
        if inputs.vin < VIN_OFF {
            // Transition to OffNoVinState
            return Ok(StateMachine::OffNoVin(OffNoVinState {}));
        }

        Ok(StateMachine::Booting(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting BootingState");
        // Cleanup state
        Ok(())
    }
}

impl State for OnState {
    async fn enter(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering OnState");
        // Set the LED blink pattern
        self.set_led_pattern(_context, &StateMachine::On(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        if inputs.vin < VIN_OFF {
            // Transition to DepletingState
            return Ok(StateMachine::Depleting(DepletingState::new()));
        }
        if !inputs.cm_on {
            return Ok(StateMachine::OffNoVin(OffNoVinState {}));
        }
        Ok(StateMachine::On(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting OnState");
        // Cleanup state
        Ok(())
    }
}

impl State for DepletingState {
    async fn enter(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering DepletingState");
        self.entry_time = Instant::now();
        // Set the LED blink pattern
        self.set_led_pattern(_context, &StateMachine::Depleting(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        let now = Instant::now();
        if now.duration_since(self.entry_time) > Duration::from_secs(5) {
            // Transition to ShutdownState
            return Ok(StateMachine::Shutdown(ShutdownState::new()));
        }

        if inputs.vin > VIN_OFF {
            // Transition to OnState
            return Ok(StateMachine::On(OnState {}));
        }
        if !inputs.cm_on {
            return Ok(StateMachine::OffNoVin(OffNoVinState {}));
        }
        Ok(StateMachine::Depleting(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting DepletingState");
        // Cleanup state
        Ok(())
    }
}

impl State for ShutdownState {
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering ShutdownState");
        // Double-click the power button
        context
            .power_button_channel
            .send(PowerButtonEvents::DoubleClick)
            .await;
        // Set the LED blink pattern
        self.set_led_pattern(context, &StateMachine::Shutdown(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        let now = Instant::now();
        if !inputs.cm_on
            || now.duration_since(self.entry_time)
                > Duration::from_millis(SHUTDOWN_WAIT_DURATION_MS as u64)
        {
            // Transition to OffState
            return Ok(StateMachine::Off(OffState::new()));
        }

        Ok(StateMachine::Shutdown(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting ShutdownState");
        // Cleanup state
        Ok(())
    }
}

impl State for OffState {
    async fn enter(&mut self, context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Entering OffState");
        context.outputs.ven.set_low();
        context.outputs.pcie_sleep.set_high();
        context.outputs.dis_usb0.set_high();
        context.outputs.dis_usb1.set_high();
        context.outputs.dis_usb2.set_high();
        context.outputs.dis_usb3.set_high();
        // Set the LED blink pattern
        self.set_led_pattern(context, &StateMachine::Off(*self))
            .await?;
        Ok(())
    }

    async fn run(&mut self, _context: &mut StateMachineContext) -> Result<StateMachine, ()> {
        let inputs = INPUTS.lock().await;
        if inputs.vin > VIN_OFF {
            // Transition to OffChargingState
            return Ok(StateMachine::OffCharging(OffChargingState {}));
        }
        Ok(StateMachine::Off(*self))
    }

    async fn exit(&mut self, _context: &mut StateMachineContext) -> Result<(), ()> {
        info!("Exiting OffState");
        // Cleanup state
        Ok(())
    }
}

#[task]
pub async fn state_machine_task(
    smor: StateMachineOutputResources,
    power_button_channel: &'static PowerButtonChannelType,
    led_blinker_channel: &'static LEDBlinkerChannelType,
) {
    // Initialize resources
    let outputs = Outputs::new(smor);

    let mut context = StateMachineContext::new(outputs, power_button_channel, led_blinker_channel);

    let mut prev_state = StateMachine::Init(InitState {});
    let mut state = prev_state;

    let mut ticker = Ticker::every(Duration::from_millis(500));

    loop {
        // Handle state machine transitions
        ticker.next().await;

        state = state.run(&mut context).await.unwrap();

        // Check if the state has changed
        if state != prev_state {
            // Exit the previous state
            prev_state.exit(&mut context).await.unwrap();
            // Enter the new state
            state.enter(&mut context).await.unwrap();
            // Update the current state
            prev_state = state;
        }
    }
}
