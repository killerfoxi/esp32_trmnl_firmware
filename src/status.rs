use smart_leds::{brightness, colors, SmartLedsWrite, RGB8};

#[derive(Debug, Clone, Copy)]
pub enum Status {
    Booting,
    BootFailure,
    Working,
    Sleeping,
    Failure,
}

impl Status {
    fn as_color(&self) -> RGB8 {
        match self {
            Self::Booting => colors::DARK_GOLDENROD,
            Self::Sleeping => colors::BLUE,
            Self::Working => colors::GREEN,
            Self::Failure => colors::RED,
            Self::BootFailure => colors::CRIMSON,
        }
    }
}

pub struct Led<LED: SmartLedsWrite> {
    brightness: u8,
    writer: LED,
}

impl<LED> Led<LED>
where
    LED: SmartLedsWrite<Color = RGB8>,
    LED::Error: core::fmt::Debug,
{
    pub fn new(led: LED, brightness: u8) -> Self {
        Self {
            writer: led,
            brightness,
        }
    }

    pub fn set_status(&mut self, status: Status) {
        self.writer
            .write(brightness([status.as_color()].into_iter(), self.brightness))
            .unwrap();
    }
}
