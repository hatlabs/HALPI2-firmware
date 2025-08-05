use alloc::boxed::Box;
use alloc::vec;
use smart_leds::colors::*;

use crate::tasks::led_blinker::*;
use crate::tasks::state_machine::State;

// Provide LED Patterns for different state machine states

pub fn get_state_pattern(state: &State) -> LEDPattern {
    match state {
        State::PowerOff {} => LEDPattern::new(vec![Box::new(Colors::new(
            100,
            [BLACK, BLACK, BLACK, BLACK, RED],
        ))]),
        State::OffCharging {} => LEDPattern::new(vec![Box::new(SupercapBar::new(1000, RED))]),
        State::SystemStartup {} => LEDPattern::new(vec![
            Box::new(RoyalRainbow::new(1280, true)),
            Box::new(OneColor::new(1000, RED)),
            Box::new(OneColor::new(1000, GREEN)),
            Box::new(OneColor::new(1000, BLUE)),
            Box::new(OneColor::new(1000, YELLOW)),
            Box::new(OneColor::new(1000, MAGENTA)),
            Box::new(OneColor::new(1000, CYAN)),
            Box::new(OneColor::new(1000, WHITE)),
            Box::new(Off::new(1000)),
        ]),
        State::Operational { co_op_enabled: false } => LEDPattern::new(vec![Box::new(SupercapBar::new(100, YELLOW))]),
        State::Operational { co_op_enabled: true } => LEDPattern::new(vec![Box::new(SupercapBar::new(100, GREEN))]),
        State::Blackout { co_op_enabled: false, .. } => {
            LEDPattern::new(vec![Box::new(SupercapBar::new(100, ORANGE))])
        }
        State::Blackout { co_op_enabled: true, .. } => {
            LEDPattern::new(vec![Box::new(SupercapBar::new(100, DARK_OLIVE_GREEN))])
        }
        State::GracefulShutdown { .. } => LEDPattern::new(vec![Box::new(SupercapBar::new(100, PURPLE))]),
        State::PoweredDown { .. } => LEDPattern::new(vec![Box::new(OneColor::new(100, BLACK))]),
        State::HostUnresponsive { .. } => LEDPattern::new(vec![Box::new(OneColor::new(100, RED))]),
        State::EnteringStandby {} => LEDPattern::new(vec![Box::new(OneColor::new(100, BLUE))]),
        State::Standby {} => LEDPattern::new(vec![Box::new(OneColor::new(100, DARK_RED))]),
    }
}

pub fn get_vscap_alarm_pattern() -> LEDPattern {
    LEDPattern::new(vec![
        Box::new(OneColor::new(100, RED)),
        Box::new(OneColor::new(100, BLACK)),
    ])
}
