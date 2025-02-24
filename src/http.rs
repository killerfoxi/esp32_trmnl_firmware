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
use esp_hal::rng::Rng;
use log::{debug, error};
use rand_core::RngCore;
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::{Method, RequestBuilder},
    response::StatusCode,
};

#[derive(Debug)]
pub enum Error {
    ConnectionReset,
    RequestTimedOut,
    Http,
    StatusCode(StatusCode),
}

impl From<tcp::Error> for Error {
    fn from(_: tcp::Error) -> Self {
        Self::ConnectionReset
    }
}

impl From<reqwless::Error> for Error {
    fn from(_: reqwless::Error) -> Self {
        Self::Http
    }
}

impl From<embassy_time::TimeoutError> for Error {
    fn from(_: embassy_time::TimeoutError) -> Self {
        Self::RequestTimedOut
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::ConnectionReset => write!(f, "connection was reset"),
            Error::Http => write!(f, "http request failed"),
            Error::RequestTimedOut => write!(f, "endpoint took to long to respond"),
            Error::StatusCode(code) => write!(f, "http request has status code of: {code:?}"),
        }
    }
}

pub trait ClientTrait {
    async fn send_request_with_header<'buf>(
        &mut self,
        buf: &'buf mut [u8],
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<&'buf [u8], Error>;
}

pub struct Client<'stack> {
    stack: Stack<'stack>,
    rng: Rng,
    tcp_client_state: TcpClientState<1, 4096, 4096>,
    rx_buf: [u8; 18 << 10],
    tx_buf: [u8; 18 << 10],
}

impl<'stack> Client<'stack> {
    pub fn new(stack: Stack<'stack>, rng: Rng) -> Self {
        Self {
            stack,
            rng,
            tcp_client_state: TcpClientState::new(),
            rx_buf: [0; 18 << 10],
            tx_buf: [0; 18 << 10],
        }
    }
}

impl ClientTrait for Client<'_> {
    async fn send_request_with_header<'buf>(
        &mut self,
        buf: &'buf mut [u8],
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<&'buf [u8], Error> {
        debug!("Sending http request to {url}");

        let tls_config = TlsConfig::new(
            self.rng.next_u64(),
            &mut self.rx_buf,
            &mut self.tx_buf,
            TlsVerify::None,
        );

        let dns_socket = DnsSocket::new(self.stack);
        let tcp_client = TcpClient::new(self.stack, &self.tcp_client_state);
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
}
