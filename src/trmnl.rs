use embedded_config::prelude::embed_config_value;
use log::debug;
use tinyqoi::Qoi;

use crate::http;

#[derive(Debug)]
pub enum Error {
    Fetch,
    Image,
}

impl From<http::Error> for Error {
    fn from(_: http::Error) -> Self {
        Self::Fetch
    }
}

impl From<tinyqoi::Error> for Error {
    fn from(_: tinyqoi::Error) -> Self {
        Self::Image
    }
}

pub trait TrmnlClient: http::ClientTrait {
    async fn fetch_image<'b>(&mut self, buf: &'b mut [u8]) -> Result<Qoi<'b>, Error> {
        let resp = self
            .send_request_with_header(
                buf,
                concat!(
                    embed_config_value!("trmnl.address"),
                    "/screen/",
                    embed_config_value!("trmnl.device_id")
                ),
                &[("Accept", "image/qoi")],
            )
            .await
            .inspect_err(|e| debug!("Received error: {e:?}"))?;
        Ok(Qoi::new(resp).inspect_err(|e| debug!("Failed to create image: {e:?}"))?)
    }
}

impl TrmnlClient for http::Client<'_> {}
