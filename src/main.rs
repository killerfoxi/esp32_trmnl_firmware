#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
#![warn(tail_expr_drop_order)]
#![warn(clippy::large_futures)]

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
use esp_hal::gpio::InputConfig;
use esp_hal::gpio::OutputConfig;
use esp_hal::gpio::{AnyPin, Input, Level, Output, Pin as _, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::peripherals::{self, TIMG0};
use esp_hal::rmt::Rmt;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Blocking, rom};
use esp_hal::{clock::CpuClock, rng::Rng};

use log::{error, info};

use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer, WithTimeout};

use esp_backtrace as _;
use static_cell::ConstStaticCell;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const WIFI_SSD: &str = embed_config_value!("wifi.ssid");
const WIFI_PWD: &str = embed_config_value!("wifi.password");

static STATUS_LED: Signal<CriticalSectionRawMutex, status::Status> = Signal::new();
static FATAL_ERROR: Signal<CriticalSectionRawMutex, Error> = Signal::new();

#[derive(Debug)]
enum Error {
    Boot,
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
    spi: peripherals::SPI2<'static>,
    rmt: peripherals::RMT<'static>,
    wifi: peripherals::WIFI<'static>,
    status_led_pin: AnyPin<'static>,
    spi_pins: SpiPins,
    display_pins: DisplayPins,
}

impl RudoPeripherals {
    fn init() -> (TimerGroup<'static, TIMG0<'static>>, peripherals::SW_INTERRUPT<'static>, Self) {
        let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
        (
            TimerGroup::new(peripherals.TIMG0),
            peripherals.SW_INTERRUPT,
            Self {
                spi: peripherals.SPI2,
                rmt: peripherals.RMT,
                wifi: peripherals.WIFI,
                status_led_pin: peripherals.GPIO8.degrade(),
                spi_pins: SpiPins {
                    clock_pin: Output::new(peripherals.GPIO19, Level::Low, OutputConfig::default()),
                    mosi_pin: Output::new(peripherals.GPIO20, Level::Low, OutputConfig::default()),
                },
                display_pins: DisplayPins {
                    cs: Output::new(peripherals.GPIO18, Level::Low, OutputConfig::default()),
                    busy: Input::new(
                        peripherals.GPIO23,
                        InputConfig::default().with_pull(Pull::Up),
                    ),
                    dc: Output::new(peripherals.GPIO21, Level::Low, OutputConfig::default()),
                    rst: Output::new(peripherals.GPIO22, Level::High, OutputConfig::default()),
                    pwr: Output::new(peripherals.GPIO10, Level::Low, OutputConfig::default()),
                },
            },
        )
    }

    async fn boot(self, spawner: &Spawner) -> Result<Rudo, BootError> {
        // Setup the status LED for indication.
        let rmt = Rmt::new(self.rmt, Rate::from_mhz(80)).unwrap();
        let led = esp_hal_smartled2::Ws2812SmartLeds::<
            { esp_hal_smartled2::buffer_size::<smart_leds::RGB8>(1) },
            Blocking,
        >::new(rmt.channel0, self.status_led_pin)
        .unwrap();
        spawner.spawn(status_led_runner(led).unwrap());
        STATUS_LED.signal(status::Status::Booting);

        let seed = (Rng::new().random() as u64) << 32 | Rng::new().random() as u64;

        info!("Connecting to wifi");
        let stack = wifi::connect(
            spawner,
            self.wifi,
            seed,
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
                .with_frequency(Rate::from_mhz(8))
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
}

static IMG_BUF: ConstStaticCell<[u8; 56 << 10]> = ConstStaticCell::new([0; 56 << 10]);

#[embassy_executor::task]
async fn status_led_runner(
    led: esp_hal_smartled2::Ws2812SmartLeds<'static, { esp_hal_smartled2::buffer_size::<smart_leds::RGB8>(1) }, Blocking>,
) {
    let mut status_led = status::Led::new(led, 10);
    loop {
        status_led.set_status(STATUS_LED.wait().await);
    }
}

#[embassy_executor::task]
async fn update_screen(mut rudo: Rudo) -> ! {
    use crate::trmnl::TrmnlClient;
    use embedded_graphics::Drawable;
    STATUS_LED.signal(status::Status::Working);

    let mut client = http::Client::new(rudo.stack);
    let buf = IMG_BUF.take();

    info!("Ready.");
    loop {
        STATUS_LED.signal(status::Status::Working);
        info!("Fetching data for screen.");
        let (image_url, sleep_dur) = match client.fetch_api_display(buf).await {
            Ok(resp) => (resp.image_url, Duration::from_secs(resp.refresh_rate)),
            Err(e) => {
                error!("Failed to fetch from /api/display: {e:?}");
                STATUS_LED.signal(status::Status::Failure);
                Timer::after_secs(30).await;
                continue;
            }
        };
        info!("Got response. Continue to fetch image from: {}", image_url);
        match client.fetch_image(buf, &image_url).await {
            Ok(img) => {
                rudo.screen.clear();
                embedded_graphics::image::Image::new(&img, Point::zero())
                    .draw(&mut rudo.screen.display().color_converted())
                    .unwrap();
                if let Err(e) = rudo.screen.update() {
                    error!("Display update failed: {e:?}");
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
        info!("Going to sleep for: {} seconds", sleep_dur.as_secs());
        STATUS_LED.signal(status::Status::Sleeping);
        Timer::after(sleep_dur).await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    esp_alloc::heap_allocator!(size: 96 << 10);

    let (timer0, sw_interrupt, rudo) = RudoPeripherals::init();
    let sw_int = SoftwareInterruptControl::new(sw_interrupt);
    esp_rtos::start(timer0.timer0, sw_int.software_interrupt0);
    info!("RTOS is initialized");

    info!("Booting...");
    match rudo.boot(&spawner).await {
        Err(e) => {
            STATUS_LED.signal(status::Status::BootFailure);
            error!("Boot failed: {e:?}");
            error!("Can't continue");
        }
        Ok(rudo) => {
            info!("Boot finished.");
            spawner.spawn(update_screen(rudo).unwrap());
            let e = FATAL_ERROR.wait().await;
            STATUS_LED.signal(status::Status::Failure);
            error!("The embedded system encountered an error: {e:?}");
            error!("Can't continue");
        }
    }

    info!("Reboot triggered.");
    Timer::after_secs(3).await;
    rom::software_reset();
}
