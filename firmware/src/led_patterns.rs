use alloc::boxed::Box;
use alloc::vec;
use smart_leds::colors::*;

use crate::tasks::led_blinker::*;
use crate::tasks::state_machine::State;

// Provide LED Patterns for different state machine states

pub fn get_state_pattern(state: &State) -> LEDPattern {
    match state {
        State::OffNoVin {} => LEDPattern::new(vec![Box::new(Colors::new(
            100,
            [BLACK, BLACK, BLACK, BLACK, RED],
        ))]),
        State::OffCharging {} => LEDPattern::new(vec![Box::new(SupercapBar::new(1000, RED))]),
        State::Booting {} => LEDPattern::new(vec![
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
        State::OnSolo {} => LEDPattern::new(vec![Box::new(SupercapBar::new(100, YELLOW))]),
        State::OnCoOp {} => LEDPattern::new(vec![Box::new(SupercapBar::new(100, GREEN))]),
        State::DepletingSolo { .. } => {
            LEDPattern::new(vec![Box::new(SupercapBar::new(100, ORANGE))])
        }
        State::DepletingCoOp {} => {
            LEDPattern::new(vec![Box::new(SupercapBar::new(100, DARK_OLIVE_GREEN))])
        }
        State::Shutdown { .. } => LEDPattern::new(vec![Box::new(SupercapBar::new(100, PURPLE))]),
        State::Off { .. } => LEDPattern::new(vec![Box::new(OneColor::new(100, BLACK))]),
        State::WatchdogAlert { .. } => LEDPattern::new(vec![Box::new(OneColor::new(100, RED))]),
        State::StandbyShutdown {} => LEDPattern::new(vec![Box::new(OneColor::new(100, BLUE))]),
        State::Standby {} => LEDPattern::new(vec![Box::new(OneColor::new(100, DARK_RED))]),
    }
}
