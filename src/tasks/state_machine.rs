use embassy_executor::task;
use embassy_time::{Duration, Timer};

#[task]
pub async fn state_machine_task() {
    loop {
        // Handle state machine transitions
        // TODO: Implement state machine logic
        Timer::after(Duration::from_millis(100)).await;
    }
}
