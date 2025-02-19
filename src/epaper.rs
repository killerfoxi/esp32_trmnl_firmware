use core::convert::Infallible;

use embedded_graphics::{pixelcolor::BinaryColor, prelude::DrawTarget};
use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
    spi::SpiBus,
};
use embedded_hal_bus::spi::ExclusiveDevice;
use epd_waveshare::{
    epd7in5_v2::{Display7in5, Epd7in5},
    prelude::WaveshareDisplay,
};
use log::info;
use static_cell::StaticCell;

#[derive(Debug)]
pub enum Error {
    Pin,
    InitScreen,
    WakeUp,
    Sleep,
    BecomingReady,
    UpdateScreen,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Pin => write!(f, "error setting cs/pwr pin"),
            Error::InitScreen => write!(f, "could not initialize screen"),
            Error::WakeUp => write!(f, "error waking up screen"),
            Error::Sleep => write!(f, "error putting screen to sleep"),
            Error::BecomingReady => write!(f, "error waiting for screen to become ready"),
            Error::UpdateScreen => write!(f, "error updating the screen"),
        }
    }
}

static FRAMEBUFFER: StaticCell<Display7in5> = StaticCell::new();

pub struct Screen<SPI, CS, BUSY, DC, RST, PWR, DELAY> {
    delay: DELAY,
    pwr_pin: PWR,
    spi_device: ExclusiveDevice<SPI, CS, DELAY>,
    device: Epd7in5<ExclusiveDevice<SPI, CS, DELAY>, BUSY, DC, RST, DELAY>,
    buffer: &'static mut Display7in5,
}

impl<SPI, CS, BUSY, DC, RST, PWR, DELAY> Screen<SPI, CS, BUSY, DC, RST, PWR, DELAY>
where
    SPI: SpiBus,
    CS: OutputPin,
    BUSY: InputPin,
    DC: OutputPin,
    RST: OutputPin,
    PWR: OutputPin,
    DELAY: DelayNs + Clone,
{
    pub fn init(
        spi_bus: SPI,
        cs_pin: CS,
        busy_pin: BUSY,
        dc_pin: DC,
        rst_pin: RST,
        pwr_pin: PWR,
        mut delay: DELAY,
    ) -> Result<Self, Error> {
        let mut device =
            ExclusiveDevice::new(spi_bus, cs_pin, delay.clone()).map_err(|_| Error::Pin)?;
        let screen = Epd7in5::new(&mut device, busy_pin, dc_pin, rst_pin, &mut delay, None)
            .map_err(|_| Error::InitScreen)?;
        Ok(Self {
            delay,
            pwr_pin,
            spi_device: device,
            device: screen,
            buffer: FRAMEBUFFER.init_with(Display7in5::default),
        })
    }
}

impl<SPI, CS, BUSY, DC, RST, PWR, DELAY> Display for Screen<SPI, CS, BUSY, DC, RST, PWR, DELAY>
where
    SPI: SpiBus,
    CS: OutputPin,
    BUSY: InputPin,
    DC: OutputPin,
    RST: OutputPin,
    PWR: OutputPin,
    DELAY: DelayNs + Clone,
{
    type Error = Error;
    type Color = epd_waveshare::color::Color;

    fn clear(&mut self) {
        self.buffer.clear(BinaryColor::On.into()).unwrap();
    }

    fn update(&mut self) -> Result<(), Self::Error> {
        self.pwr_pin.set_high().map_err(|_| Error::Pin)?;
        self.device
            .wake_up(&mut self.spi_device, &mut self.delay)
            .map_err(|_| Error::WakeUp)?;
        info!("Waiting for display to awake...");
        self.device
            .wait_until_idle(&mut self.spi_device, &mut self.delay)
            .map_err(|_| Error::BecomingReady)?;
        info!("Display reporting ready... updating the content.");
        self.device
            .update_and_display_frame(&mut self.spi_device, self.buffer.buffer(), &mut self.delay)
            .map_err(|_| Error::UpdateScreen)?;
        info!("Putting screen back to sleep.");
        self.device
            .sleep(&mut self.spi_device, &mut self.delay)
            .map_err(|_| Error::Sleep)?;
        self.pwr_pin.set_low().map_err(|_| Error::Pin)
    }

    fn display(&mut self) -> &mut impl DrawTarget<Color = Self::Color, Error = Infallible> {
        self.buffer
    }
}

pub trait Display {
    type Error;
    type Color;

    fn clear(&mut self);
    fn update(&mut self) -> Result<(), Self::Error>;
    fn display(&mut self) -> &mut impl DrawTarget<Color = Self::Color, Error = Infallible>;
}
