use alloc::boxed::Box;
use alloc::vec;
use smart_leds::colors::*;

use crate::tasks::led_blinker::*;
use crate::tasks::state_machine::TargetState;

// Provide LED Patterns for different state machine states

pub fn get_state_pattern(state: &TargetState) -> LEDPattern {
    match state {
        TargetState::OffNoVin => LEDPattern::new(vec![Box::new(Colors::new(
            100,
            [RED, BLACK, BLACK, BLACK, BLACK],
        ))]),
        TargetState::OffCharging => LEDPattern::new(vec![Box::new(OneColor::new(1000, RED))]),
        TargetState::Booting => LEDPattern::new(vec![
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
        TargetState::On => LEDPattern::new(vec![Box::new(OneColor::new(100, GREEN))]),
        TargetState::Depleting => LEDPattern::new(vec![Box::new(OneColor::new(100, YELLOW))]),
        TargetState::Shutdown => LEDPattern::new(vec![Box::new(OneColor::new(100, PURPLE))]),
        TargetState::Off => LEDPattern::new(vec![Box::new(OneColor::new(100, BLACK))]),
        _ => todo!(),
    }
}
