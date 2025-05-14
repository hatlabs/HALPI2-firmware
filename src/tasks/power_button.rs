use defmt::{debug, info};
use embassy_executor::task;
use embassy_rp::gpio::{Level, Output};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel};
use embassy_time::{Duration, Timer};

use crate::config_resources::PowerButtonResources;


pub enum PowerButtonEvents {
    Press,
    Release,
    Click,
    DoubleClick,
    LongPress,
}

pub type PowerButtonChannelType = channel::Channel<CriticalSectionRawMutex, PowerButtonEvents, 8>;
pub static POWER_BUTTON_EVENT_CHANNEL: PowerButtonChannelType = channel::Channel::new();

#[task]
pub async fn power_button_output_task(r: PowerButtonResources) {
    info!("Initializing power button output task");
    let mut button = Output::new(r.pin, Level::High);

    let receiver = POWER_BUTTON_EVENT_CHANNEL.receiver();

    info!("Power button output task initialized");

    loop {
        let event = receiver.receive().await;
        let event_string = match event {
            PowerButtonEvents::Press => "Press",
            PowerButtonEvents::Release => "Release",
            PowerButtonEvents::Click => "Click",
            PowerButtonEvents::DoubleClick => "DoubleClick",
            PowerButtonEvents::LongPress => "LongPress",
        };
        debug!("Received event: {:?}", event_string);
        match event {
            PowerButtonEvents::Press => {
                button.set_low();
            }
            PowerButtonEvents::Release => {
                button.set_high();
            }
            PowerButtonEvents::Click => {
                // Ensure that the button is released before handling the click event
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
                button.set_low();
                Timer::after(Duration::from_millis(200)).await;
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
            }
            PowerButtonEvents::DoubleClick => {
                // Handle double click event
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
                button.set_low();
                Timer::after(Duration::from_millis(200)).await;
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
                button.set_low();
                Timer::after(Duration::from_millis(200)).await;
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
            }
            PowerButtonEvents::LongPress => {
                // Handle long press event (depress for 5.5 seconds)
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
                button.set_low();
                Timer::after(Duration::from_millis(5500)).await;
                button.set_high();
                Timer::after(Duration::from_millis(100)).await;
            }
        }
    }
}
