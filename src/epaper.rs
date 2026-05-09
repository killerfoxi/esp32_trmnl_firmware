use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::pixelcolor::BinaryColor;
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
    #[allow(dead_code)]
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

    /// Wake the display, wait for it to become ready, and send the framebuffer content.
    fn wake_and_display(&mut self) -> Result<(), Error> {
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
            .map_err(|_| Error::UpdateScreen)
    }

    // Safe to unwrap: Display7in5::clear() just fills an in-memory buffer and cannot fail.
    pub fn clear(&mut self) {
        self.buffer.clear(BinaryColor::On.into()).unwrap();
    }

    pub fn update(&mut self) -> Result<(), Error> {
        self.pwr_pin.set_high().map_err(|_| Error::Pin)?;

        let result = self.wake_and_display();

        // Always attempt to sleep the display and power down, even on error,
        // to avoid leaving the e-paper panel in an active power state.
        info!("Putting screen back to sleep.");
        let _ = self.device.sleep(&mut self.spi_device, &mut self.delay);
        let _ = self.pwr_pin.set_low();

        result
    }

    pub fn display(&mut self) -> &mut Display7in5 {
        self.buffer
    }
}
