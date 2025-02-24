use embassy_time::Duration;
use embedded_config::prelude::embed_config_value;
use log::debug;
use reqwless::response::StatusCode;
use serde::Deserialize;
use tinyqoi::Qoi;

use crate::http;

#[derive(Debug)]
pub enum Error {
    Fetch,
    UnexpectedStatus(StatusCode),
    Image,
    Decode,
}

impl From<http::Error> for Error {
    fn from(err: http::Error) -> Self {
        match err {
            http::Error::ConnectionReset | http::Error::RequestTimedOut | http::Error::Http => {
                Error::Fetch
            }
            http::Error::StatusCode(status_code) => Error::UnexpectedStatus(status_code),
        }
    }
}

impl From<tinyqoi::Error> for Error {
    fn from(_: tinyqoi::Error) -> Self {
        Self::Image
    }
}

impl From<serde_json_core::de::Error> for Error {
    fn from(_: serde_json_core::de::Error) -> Self {
        Error::Decode
    }
}

#[derive(Deserialize)]
pub struct ApiResponse {
    pub image_url: heapless::String<128>,
    pub refresh_rate: u64,
}

macro_rules! url {
    ($path:expr) => {
        concat!(embed_config_value!("trmnl.address"), "/", $path)
    };
}

pub trait TrmnlClient: http::ClientTrait {
    async fn fetch_api_display(&mut self, buf: &mut [u8]) -> Result<ApiResponse, Error> {
        let resp = self
            .send_request_with_header(
                buf,
                url!("api/display"),
                &[("Access-Token", embed_config_value!("trmnl.device_id"))],
            )
            .await
            .inspect_err(|e| debug!("Failed to fetch api response: {e:?}"))?;
        let (api, _) = serde_json_core::from_slice(resp)?;
        Ok(api)
    }

    async fn fetch_image<'b>(&mut self, buf: &'b mut [u8], url: &str) -> Result<Qoi<'b>, Error> {
        let resp = self
            .send_request_with_header(buf, url, &[("Accept", "image/qoi")])
            .await
            .inspect_err(|e| debug!("Received error: {e:?}"))?;
        Ok(Qoi::new(resp).inspect_err(|e| debug!("Failed to create image: {e:?}"))?)
    }
}

impl TrmnlClient for http::Client<'_> {}
