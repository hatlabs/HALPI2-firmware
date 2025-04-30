use alloc::boxed::Box;
use alloc::vec;
use smart_leds::RGB8;
use smart_leds::colors::*;

use crate::tasks::led_blinker::*;
use crate::tasks::state_machine::StateMachine;

// Provide LED Patterns for different state machine states

pub fn get_state_pattern(state: &StateMachine) -> LEDPattern {
    match state {
        StateMachine::OffNoVin(_) => LEDPattern::new(vec![Box::new(Colors::new(
            100,
            [RED, BLACK, BLACK, BLACK, BLACK],
        ))]),
        StateMachine::OffCharging(_) => LEDPattern::new(vec![Box::new(OneColor::new(1000, RED))]),
        StateMachine::Booting(_) => LEDPattern::new(vec![
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
        StateMachine::On(_) => LEDPattern::new(vec![Box::new(OneColor::new(100, GREEN))]),
        StateMachine::Depleting(_) => LEDPattern::new(vec![Box::new(OneColor::new(100, YELLOW))]),
        StateMachine::Shutdown(_) => LEDPattern::new(vec![Box::new(OneColor::new(100, PURPLE))]),
        StateMachine::Off(_) => LEDPattern::new(vec![Box::new(OneColor::new(100, BLACK))]),
        _ => todo!(),
    }
}
