use alloc::boxed::Box;
use alloc::vec;
use smart_leds::colors::*;

use crate::tasks::led_blinker::*;
use crate::tasks::state_machine::HalpiStates;

// Provide LED Patterns for different state machine states

pub fn get_state_pattern(state: &HalpiStates) -> LEDPattern {
    match state {
        HalpiStates::OffNoVin => LEDPattern::new(vec![Box::new(Colors::new(
            100,
            [BLACK, BLACK, BLACK, BLACK, RED],
        ))]),
        HalpiStates::OffCharging => LEDPattern::new(vec![Box::new(SupercapBar::new(1000, RED))]),
        HalpiStates::Booting => LEDPattern::new(vec![
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
        HalpiStates::OnSolo => LEDPattern::new(vec![Box::new(SupercapBar::new(100, YELLOW))]),
        HalpiStates::OnCoOp => LEDPattern::new(vec![Box::new(SupercapBar::new(100, GREEN))]),
        HalpiStates::DepletingSolo => LEDPattern::new(vec![Box::new(SupercapBar::new(100, ORANGE))]),
        HalpiStates::DepletingCoOp => LEDPattern::new(vec![Box::new(SupercapBar::new(100, DARK_OLIVE_GREEN))]),
        HalpiStates::Shutdown => LEDPattern::new(vec![Box::new(SupercapBar::new(100, PURPLE))]),
        HalpiStates::Off => LEDPattern::new(vec![Box::new(OneColor::new(100, BLACK))]),
        HalpiStates::WatchdogAlert => LEDPattern::new(vec![Box::new(OneColor::new(100, RED))]),
        HalpiStates::StandbyShutdown => LEDPattern::new(vec![Box::new(OneColor::new(100, BLUE))]),
        HalpiStates::Standby => LEDPattern::new(vec![Box::new(OneColor::new(100, DARK_RED))]),
    }
}
