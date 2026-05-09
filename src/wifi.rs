use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_radio::wifi::{
    self, ConnectedStationInfo, ControllerConfig, WifiController, WifiError,
};
use log::{debug, error, info};
use static_cell::StaticCell;

static STACK_RESOURCES: StaticCell<StackResources<2>> = StaticCell::new();

pub async fn connect(
    spawner: &Spawner,
    wifi: WIFI<'static>,
    seed: u64,
    (ssid, password): (&str, &str),
) -> Result<Stack<'static>, Error> {
    let (controller, interfaces) = wifi::new(wifi, ControllerConfig::default())?;

    let config = embassy_net::Config::dhcpv4(embassy_net::DhcpConfig::default());

    debug!("Initialize network stack");
    let stack_resources: &'static mut _ = STACK_RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(interfaces.station, config, stack_resources, seed);

    let ssid: String = ssid.into();
    let password: String = password.into();
    spawner.spawn(connection(controller, ssid, password).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    debug!("Wait for network link");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    debug!("Wait for IP address");
    loop {
        if let Some(config) = stack.config_v4() {
            info!(
                "Connected to WiFi with IP address {}; gw {:?}; dns_servers: {:?}",
                config.address, config.gateway, config.dns_servers
            );
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    Ok(stack)
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>, ssid: String, password: String) {
    if let Err(error) = connection_fallible(&mut controller, ssid, password).await {
        error!("Cannot connect to WiFi: {error:?}");
    }
}

async fn connection_fallible(
    controller: &mut WifiController<'static>,
    ssid: String,
    password: String,
) -> Result<(), Error> {
    debug!("Start connection");
    let client_config = wifi::Config::Station(
        wifi::sta::StationConfig::default()
            .with_ssid(ssid)
            .with_password(password),
    );
    controller.set_config(&client_config)?;

    loop {
        if controller.is_connected() {
            // Wait until we're no longer connected
            let mut subscriber = controller.subscribe()?;
            loop {
                match subscriber.next_event().await {
                    esp_radio::wifi::event::MessageResult::Message(
                        esp_radio::wifi::event::EventInfo::StationDisconnected { .. },
                    ) => break,
                    _ => {}
                }
            }
            debug!("Disconnected, reconnecting...");
        }

        debug!("Connect to WiFi network");
        match controller.connect_async().await {
            Ok(ConnectedStationInfo { .. }) => {
                debug!("Connected to WiFi network");
            }
            Err(error) => {
                error!("Failed to connect to WiFi network: {error:?}");
                Timer::after(Duration::from_secs(5)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, esp_radio::wifi::Interface<'static>>) {
    runner.run().await
}

#[derive(Debug)]
pub enum Error {
    Operation,
}

impl From<WifiError> for Error {
    fn from(e: WifiError) -> Self {
        debug!("Discarding WiFi error details: {e:?}");
        Self::Operation
    }
}
