use core::fmt::Display;

use embassy_net::{
    Stack,
    dns::DnsSocket,
    tcp::{
        self,
        client::{TcpClient, TcpClientState},
    },
};
use embassy_time::{Duration, WithTimeout};
use embedded_config::prelude::embed_config_value;
use esp_hal::rng::Rng;
use log::{debug, error};
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::{Method, RequestBuilder},
    response::StatusCode,
};
use serde::Deserialize;
use static_cell::StaticCell;
use tinyqoi::Qoi;

#[derive(Debug)]
pub enum Error {
    ConnectionReset,
    RequestTimedOut,
    Http,
    StatusCode(StatusCode),
    Decode,
    Image,
}

impl From<tcp::Error> for Error {
    fn from(e: tcp::Error) -> Self {
        debug!("Discarding TCP error details: {e:?}");
        Self::ConnectionReset
    }
}

impl From<reqwless::Error> for Error {
    fn from(e: reqwless::Error) -> Self {
        debug!("Discarding HTTP error details: {e:?}");
        Self::Http
    }
}

impl From<embassy_time::TimeoutError> for Error {
    fn from(_: embassy_time::TimeoutError) -> Self {
        Self::RequestTimedOut
    }
}

impl From<serde_json_core::de::Error> for Error {
    fn from(e: serde_json_core::de::Error) -> Self {
        debug!("Discarding decode error details: {e:?}");
        Self::Decode
    }
}

impl From<tinyqoi::Error> for Error {
    fn from(e: tinyqoi::Error) -> Self {
        debug!("Discarding image error details: {e:?}");
        Self::Image
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::ConnectionReset => write!(f, "connection was reset"),
            Error::Http => write!(f, "http request failed"),
            Error::RequestTimedOut => write!(f, "endpoint took too long to respond"),
            Error::StatusCode(code) => write!(f, "http request has status code of: {code:?}"),
            Error::Decode => write!(f, "failed to decode response"),
            Error::Image => write!(f, "failed to decode image"),
        }
    }
}

macro_rules! url {
    ($path:expr) => {
        concat!(embed_config_value!("trmnl.address"), "/", $path)
    };
}

#[derive(Deserialize)]
pub struct ApiResponse {
    pub image_url: heapless::String<128>,
    pub refresh_rate: u64,
}

static TCP_STATE: StaticCell<TcpClientState<1, 2048, 2048>> = StaticCell::new();
static RX_BUF: StaticCell<[u8; 16 << 10]> = StaticCell::new();
static TX_BUF: StaticCell<[u8; 16 << 10]> = StaticCell::new();

pub struct Client<'stack> {
    stack: Stack<'stack>,
    tcp_client_state: &'static mut TcpClientState<1, 2048, 2048>,
    rx_buf: &'static mut [u8; 16 << 10],
    tx_buf: &'static mut [u8; 16 << 10],
    rng: Rng,
}

impl<'stack> Client<'stack> {
    pub fn new(stack: Stack<'stack>) -> Self {
        Self {
            stack,
            tcp_client_state: TCP_STATE.init(TcpClientState::new()),
            rx_buf: RX_BUF.init([0; 16 << 10]),
            tx_buf: TX_BUF.init([0; 16 << 10]),
            rng: Rng::new(),
        }
    }

    async fn send_request_with_header<'buf>(
        &mut self,
        buf: &'buf mut [u8],
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<&'buf [u8], Error> {
        debug!("Sending http request to {url}");

        let seed = (self.rng.random() as u64) << 32 | self.rng.random() as u64;
        let tls_config = TlsConfig::new(
            seed,
            self.rx_buf,
            self.tx_buf,
            TlsVerify::None,
        );

        let dns_socket = DnsSocket::new(self.stack);
        let tcp_client = TcpClient::new(self.stack, self.tcp_client_state);
        let mut client = HttpClient::new_with_tls(&tcp_client, &dns_socket, tls_config);

        debug!("Creating request");
        debug!("Setting headers: {headers:?}");
        let mut req = client
            .request(Method::GET, url)
            .await
            .inspect(|_| debug!("Request prepped."))
            .inspect_err(|e| error!("Producing request: {e:?}"))?
            .headers(headers);
        debug!("Sending request");
        let resp = match req.send(buf).with_timeout(Duration::from_secs(45)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                error!("http request failed with: {e:?}");
                return Err(Error::Http);
            }
            Err(_) => return Err(Error::RequestTimedOut),
        };
        if !resp.status.is_successful() {
            return Err(Error::StatusCode(resp.status));
        }
        let buf = resp
            .body()
            .read_to_end()
            .await
            .inspect_err(|e| error!("Failed to read full body: {e:?}"))?;
        debug!("Received {} bytes", buf.len());
        Ok(buf)
    }

    pub async fn fetch_api_display(&mut self, buf: &mut [u8]) -> Result<ApiResponse, Error> {
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

    pub async fn fetch_image<'b>(&mut self, buf: &'b mut [u8], url: &str) -> Result<Qoi<'b>, Error> {
        let resp = self
            .send_request_with_header(buf, url, &[("Accept", "image/qoi")])
            .await
            .inspect_err(|e| debug!("Failed to fetch image: {e:?}"))?;
        Ok(Qoi::new(resp).inspect_err(|e| debug!("Failed to decode image: {e:?}"))?)
    }
}
