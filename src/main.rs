#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]

mod epaper;
mod http;
mod status;
mod trmnl;
mod wifi;

use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embedded_config::prelude::*;
use embedded_graphics::prelude::{DrawTargetExt, Point};
use epaper::Display;
use esp_hal::gpio::{AnyPin, Input, Level, Output, Pin, Pull};
use esp_hal::peripherals::{self, Peripherals, TIMG0};
use esp_hal::rmt::{ChannelCreator, Rmt};
use esp_hal::spi::master::Spi;
use esp_hal::time::RateExtU32;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{clock::CpuClock, rng::Rng};
use esp_hal::{reset, Blocking};

use esp_hal_smartled::smartLedBuffer;
use log::{error, info};

use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer, WithTimeout};

use esp_backtrace as _;
use static_cell::StaticCell;

extern crate alloc;

const WIFI_SSD: &str = embed_config_value!("wifi.ssid");
const WIFI_PWD: &str = embed_config_value!("wifi.password");

static STATUS_LED: Signal<CriticalSectionRawMutex, status::Status> = Signal::new();

#[derive(Debug)]
enum Error {
    Boot,
    Operation,
}

impl From<BootError> for Error {
    fn from(_: BootError) -> Self {
        Error::Boot
    }
}

#[derive(Debug)]
enum BootError {
    WifiConnection,
    WifiConnectionTimeout,
    SpiInit,
    ScreenInit,
}

impl From<embassy_time::TimeoutError> for BootError {
    fn from(_: embassy_time::TimeoutError) -> Self {
        Self::WifiConnectionTimeout
    }
}

struct SpiPins {
    clock_pin: Output<'static>,
    mosi_pin: Output<'static>,
}

struct DisplayPins {
    busy: Input<'static>,
    dc: Output<'static>,
    rst: Output<'static>,
    pwr: Output<'static>,
    cs: Output<'static>,
}

struct RudoPeripherals {
    spi: peripherals::SPI2,
    rmt: peripherals::RMT,
    timer0: TimerGroup<TIMG0>,
    rng: Rng,
    wifi: peripherals::WIFI,
    radio_clk: peripherals::RADIO_CLK,
    status_led_pin: AnyPin,
    spi_pins: SpiPins,
    display_pins: DisplayPins,
}

impl RudoPeripherals {
    fn init(peripherals: Peripherals) -> (Self, peripherals::SYSTIMER) {
        (
            Self {
                spi: peripherals.SPI2,
                rmt: peripherals.RMT,
                timer0: TimerGroup::new(peripherals.TIMG0),
                rng: Rng::new(peripherals.RNG),
                wifi: peripherals.WIFI,
                radio_clk: peripherals.RADIO_CLK,
                status_led_pin: peripherals.GPIO8.degrade(),
                spi_pins: SpiPins {
                    clock_pin: Output::new(peripherals.GPIO19, Level::Low),
                    mosi_pin: Output::new(peripherals.GPIO20, Level::Low),
                },
                display_pins: DisplayPins {
                    cs: Output::new(peripherals.GPIO18, Level::Low),
                    busy: Input::new(peripherals.GPIO23, Pull::Up),
                    dc: Output::new(peripherals.GPIO21, Level::Low),
                    rst: Output::new(peripherals.GPIO22, Level::High),
                    pwr: Output::new(peripherals.GPIO10, Level::Low),
                },
            },
            peripherals.SYSTIMER,
        )
    }

    async fn boot(self, spawner: &Spawner) -> Result<Rudo, BootError> {
        // Setup the status LED for indication.
        let rmt = Rmt::new(self.rmt, 80u32.MHz()).unwrap();
        spawner.must_spawn(status_led_runner(rmt.channel0, self.status_led_pin));
        STATUS_LED.signal(status::Status::Booting);

        info!("Connecting to wifi");
        let stack = wifi::connect(
            spawner,
            self.timer0.timer0,
            self.rng,
            self.wifi,
            self.radio_clk,
            (WIFI_SSD, WIFI_PWD),
        )
        .with_timeout(Duration::from_secs(20))
        .await?
        .map_err(|_| BootError::WifiConnection)?;
        info!("Wifi successfully connected");

        info!("Initializing SPI");
        let spi = Spi::new(
            self.spi,
            esp_hal::spi::master::Config::default()
                .with_frequency(8u32.MHz())
                .with_mode(esp_hal::spi::Mode::_0),
        )
        .map_err(|_| BootError::SpiInit)?
        .with_sck(self.spi_pins.clock_pin)
        .with_mosi(self.spi_pins.mosi_pin);
        info!("SPI initialized");

        info!("Initialize e-paper screen");
        let DisplayPins {
            busy: busy_pin,
            dc: dc_pin,
            rst: rst_pin,
            pwr: pwr_pin,
            cs: cs_pin,
        } = self.display_pins;
        let screen = epaper::Screen::init(spi, cs_pin, busy_pin, dc_pin, rst_pin, pwr_pin, Delay)
            .map_err(|_| BootError::ScreenInit)?;
        info!("e-paper screen initialized.");

        Ok(Rudo {
            screen,
            stack,
            rng: self.rng,
        })
    }
}

struct Rudo {
    screen: epaper::Screen<
        Spi<'static, Blocking>,
        Output<'static>,
        Input<'static>,
        Output<'static>,
        Output<'static>,
        Output<'static>,
        Delay,
    >,
    stack: Stack<'static>,
    rng: Rng,
}

#[embassy_executor::task]
async fn status_led_runner(rmt_channel: ChannelCreator<Blocking, 0>, led_pin: AnyPin) {
    let mut status_led = status::Led::new(
        esp_hal_smartled::SmartLedsAdapter::new(rmt_channel, led_pin, smartLedBuffer!(1)),
        10,
    );
    loop {
        status_led.set_status(STATUS_LED.wait().await);
    }
}

static IMG_BUF: StaticCell<[u8; 32 << 10]> = StaticCell::new();

async fn main_fallible(mut rudo: Rudo, _spawner: Spawner) -> Result<(), Error> {
    use crate::trmnl::TrmnlClient;
    use embedded_graphics::Drawable;
    STATUS_LED.signal(status::Status::Working);

    let mut client = http::Client::new(rudo.stack, rudo.rng);
    let buf = IMG_BUF.init([0; 32 << 10]);

    info!("Ready.");
    loop {
        STATUS_LED.signal(status::Status::Working);
        info!("Fetching image.");
        match client.fetch_image(buf).await {
            Ok(img) => {
                rudo.screen.clear();
                embedded_graphics::image::Image::new(&img, Point::zero())
                    .draw(&mut rudo.screen.display().color_converted())
                    .unwrap();
                if let Err(e) = rudo.screen.update() {
                    error!("Displa update failed: {e:?}");
                    STATUS_LED.signal(status::Status::Failure);
                    Timer::after_secs(2).await;
                    continue;
                }
            }
            Err(e) => {
                error!("Failed to fetch and display image: {e:?}");
                STATUS_LED.signal(status::Status::Failure);
                Timer::after_secs(25).await;
                continue;
            }
        }
        STATUS_LED.signal(status::Status::Sleeping);
        Timer::after_secs(embed_config_value!("rudo.refresh_interval_secs") as u64).await
    }
}

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(72 << 10);

    let (rudo, systimer) = RudoPeripherals::init(peripherals);
    let systimer = esp_hal::timer::systimer::SystemTimer::new(systimer);
    esp_hal_embassy::init(systimer.alarm0);
    info!("embassy is initialized");

    info!("Booting...");
    match rudo.boot(&spawner).await {
        Err(e) => {
            STATUS_LED.signal(status::Status::BootFailure);
            error!("Boot failed: {e:?}");
            error!("Can't continue");
        }
        Ok(rudo) => {
            info!("Boot finished.");
            if let Err(e) = main_fallible(rudo, spawner).await {
                STATUS_LED.signal(status::Status::Failure);
                error!("The embedded system encountered an error: {e:?}");
                error!("Can't continue");
            }
        }
    }

    info!("Reboot triggered.");
    Timer::after_secs(3).await;
    reset::software_reset();
}
