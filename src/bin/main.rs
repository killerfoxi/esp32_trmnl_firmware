#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::timer::systimer::SystemTimer;
use esp_hal::timer::timg::TimerGroup;

use log::info;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};

use esp_backtrace as _;

extern crate alloc;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(72 * 1024);

    let timer0 = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(timer0.alarm0);

    info!("Embassy initialized!");

    let timer1 = TimerGroup::new(peripherals.TIMG0);
    let _init = esp_wifi::init(
        timer1.timer0,
        esp_hal::rng::Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
    )
    .unwrap();

    // TODO: Spawn some tasks
    let _ = spawner;

    loop {
        info!("Hello world!");
        Timer::after(Duration::from_secs(1)).await;
    }
}
