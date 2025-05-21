use defmt::*;
use embassy_executor::task;
use embassy_rp::{
    adc::{Adc, Channel, Config, InterruptHandler},
    bind_interrupts,
    gpio::{Input, Pull},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Duration, Ticker};

use crate::{
    config::{VIN_MAX_VALUE, VSCAP_MAX_VALUE}, config_resources::{AnalogInputResources, DigitalInputResources, PowerButtonInputResources, UserButtonInputResources}, tasks::power_button::POWER_BUTTON_EVENT_CHANNEL
};

use super::power_button::{PowerButtonChannelType, PowerButtonEvents};

/// Input values that are read by the io_task and consumed by other tasks.
#[derive(Clone, Format)]
pub struct Inputs {
    pub vin: f32,
    pub vscap: f32,
    pub iin: f32,
    pub cm_on: bool,
    pub mcu_temp: f32,
    pub pcb_temp: f32,
    pub led_pwr: bool,
    pub led_active: bool,
    pub pg_5v: bool,
    pub pwr_btn: bool,
    pub user_btn: bool,
}

impl Inputs {
    const fn new() -> Self {
        Self {
            vin: 0.,
            vscap: 0.,
            iin: 0.,
            cm_on: false,
            mcu_temp: 0.,
            pcb_temp: 0.,
            led_pwr: false,
            led_active: false,
            pg_5v: false,
            pwr_btn: true,
            user_btn: true,
        }
    }
}

/// Shared inputs protected by a mutex.
pub static INPUTS: Mutex<CriticalSectionRawMutex, Inputs> = Mutex::new(Inputs::new());

#[task]
pub async fn power_button_input_task(
    r: PowerButtonInputResources,
) {
    info!("Starting power button input task");

    let mut button = Input::new(r.pin, Pull::Up);

    info!("Power button input task initialized");

    loop {
        button.wait_for_any_edge().await;
        debug!("Power button event detected");
        let mut inputs = INPUTS.lock().await;
        // Update the power button input state
        inputs.pwr_btn = button.is_high();
        // Send the event to the channel
        POWER_BUTTON_EVENT_CHANNEL.send(PowerButtonEvents::Release).await;
        if inputs.pwr_btn {
        } else {
            POWER_BUTTON_EVENT_CHANNEL.send(PowerButtonEvents::Press).await;
        }
    }
}

#[task]
pub async fn user_button_input_task(r: UserButtonInputResources) {
    info!("Starting user button input task");

    let mut button = Input::new(r.pin, Pull::Up);

    info!("User button input task initialized");

    loop {
        button.wait_for_any_edge().await;
        let mut inputs = INPUTS.lock().await;
        // Update the user button input state
        inputs.user_btn = button.is_high();
    }
}

#[task]
pub async fn digital_input_task(r: DigitalInputResources) {
    info!("Starting digital input task");
    // Initialize the peripherals and GPIO pins
    let led_pwr = Input::new(r.led_pwr, Pull::Up);
    let led_active = Input::new(r.led_active, Pull::Up);
    let pg_5v = Input::new(r.pg_5v, Pull::Up);
    let cm_on = Input::new(r.cm_on, Pull::Down);

    let mut ticker = Ticker::every(Duration::from_millis(10));

    info!("Digital input task initialized");

    loop {
        ticker.next().await;
        trace!("Reading digital inputs");
        let mut inputs = INPUTS.lock().await;

        // Read the input values
        inputs.led_pwr = led_pwr.is_high();
        inputs.led_active = led_active.is_high();
        inputs.pg_5v = pg_5v.is_high();
        inputs.cm_on = cm_on.is_high();
        trace!(
            "LED_PWR: {}, LED_ACTIVE: {}, PG_5V: {}, CM_ON: {}",
            inputs.led_pwr, inputs.led_active, inputs.pg_5v, inputs.cm_on
        );
    }
}

const VIN_ADC_SCALE: f32 = VIN_MAX_VALUE / 4096.0; // Scale factor for Vin readings
const VSCAP_ADC_SCALE: f32 = VSCAP_MAX_VALUE / 4096.0; // Scale factor for Vscap readings
const IIN_ADC_SCALE: f32 = 3.3 / 4096.0;

bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => InterruptHandler;
});

#[task]
pub async fn analog_input_task(r: AnalogInputResources) {
    info!("Starting analog input task");
    // Initialize the peripherals and GPIO pins
    let mut adc = Adc::new(r.adc, Irqs, Config::default());
    let mut vins = Channel::new_pin(r.vin_s, Pull::None);
    let mut vscaps = Channel::new_pin(r.vscap_s, Pull::None);
    let mut iin = Channel::new_pin(r.iin, Pull::None);
    let mut mcu_temp = Channel::new_temp_sensor(r.temp_sensor);

    let mut ticker = Ticker::every(Duration::from_millis(20));

    info!("Analog input task initialized");

    loop {
        ticker.next().await;

        trace!("Reading analog inputs");

        let vin = adc.read(&mut vins).await;
        let vscap_value = adc.read(&mut vscaps).await;
        let iin_value = adc.read(&mut iin).await;
        let mcu_temp_value = adc.read(&mut mcu_temp).await;

        let mut inputs = INPUTS.lock().await;
        inputs.vin = (vin.unwrap_or(0) as f32) * VIN_ADC_SCALE;
        inputs.vscap = (vscap_value.unwrap_or(0) as f32) * VSCAP_ADC_SCALE;
        inputs.iin = (iin_value.unwrap_or(0) as f32) * IIN_ADC_SCALE;
        // Convert to Kelvin
        inputs.mcu_temp =
            27.0 - (mcu_temp_value.unwrap_or(0) as f32 * 3.3 / 4096.0 - 0.706) / 0.001721 + 273.15;

        trace!(
            "VIN: {}, VSCAP: {}, IIN: {}",
            inputs.vin, inputs.vscap, inputs.iin
        );
    }
}
