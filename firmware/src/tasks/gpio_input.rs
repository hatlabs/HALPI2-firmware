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

use super::power_button::{PowerButtonEvents};

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

const AVERAGE_SAMPLES: usize = 10;

struct AveragedInput {
    samples: [f32; AVERAGE_SAMPLES],
    index: usize,
    sum: f32,
    count: usize,
}

impl AveragedInput {
    fn new() -> Self {
        Self {
            samples: [0.0; AVERAGE_SAMPLES],
            index: 0,
            sum: 0.0,
            count: 0,
        }
    }

    fn add_sample(&mut self, value: f32) {
        if self.count < AVERAGE_SAMPLES {
            self.samples[self.index] = value;
            self.sum += value;
            self.count += 1;
        } else {
            self.sum -= self.samples[self.index];
            self.samples[self.index] = value;
            self.sum += value;
        }
        self.index = (self.index + 1) % AVERAGE_SAMPLES;
    }

    fn average(&self) -> f32 {
        if self.count == 0 {
            0.0
        } else {
            self.sum / self.count as f32
        }
    }
}

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

    let mut vin_avg = AveragedInput::new();
    let mut vscap_avg = AveragedInput::new();
    let mut iin_avg = AveragedInput::new();
    let mut mcu_temp_avg = AveragedInput::new();

    let vin_adc_scale = crate::tasks::config_manager::get_vin_correction_scale().await * VIN_ADC_SCALE;
    let vscap_adc_scale = crate::tasks::config_manager::get_vscap_correction_scale().await * VSCAP_ADC_SCALE;
    let iin_adc_scale = crate::tasks::config_manager::get_iin_correction_scale().await * IIN_ADC_SCALE;

    loop {
        ticker.next().await;

        trace!("Reading analog inputs");

        let vin = adc.read(&mut vins).await.unwrap_or(0);
        let vscap_value = adc.read(&mut vscaps).await.unwrap_or(0);
        let iin_value = adc.read(&mut iin).await.unwrap_or(0);
        let mcu_temp_value = adc.read(&mut mcu_temp).await.unwrap_or(0);

        let vin_sample = (vin as f32) * vin_adc_scale;
        let vscap_sample = (vscap_value as f32) * vscap_adc_scale;
        let iin_sample = (iin_value as f32) * iin_adc_scale;
        let mcu_temp_sample =
            27.0 - (mcu_temp_value as f32 * 3.3 / 4096.0 - 0.706) / 0.001721 + 273.15;

        vin_avg.add_sample(vin_sample);
        vscap_avg.add_sample(vscap_sample);
        iin_avg.add_sample(iin_sample);
        mcu_temp_avg.add_sample(mcu_temp_sample);

        let mut inputs = INPUTS.lock().await;
        inputs.vin = vin_avg.average();
        inputs.vscap = vscap_avg.average();
        inputs.iin = iin_avg.average();
        inputs.mcu_temp = mcu_temp_avg.average();

        trace!(
            "VIN: {}, VSCAP: {}, IIN: {}",
            inputs.vin, inputs.vscap, inputs.iin
        );
    }
}
