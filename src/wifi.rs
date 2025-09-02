use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::{Config, DhcpConfig, Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::peripherals::WIFI;
use esp_wifi::EspWifiTimerSource;
use esp_wifi::{
    EspWifiController, InitializationError as WifiInitializationError,
    wifi::{
        ClientConfiguration, Configuration, WifiController, WifiDevice, WifiError as EspWifiError,
        WifiEvent, WifiState,
    },
};
//use heapless::String;
use log::{debug, error, info};
use static_cell::StaticCell;

static WIFI_CONTROLLER: StaticCell<EspWifiController<'static>> = StaticCell::new();

static STACK_RESOURCES: StaticCell<StackResources<8>> = StaticCell::new();

pub static STOP_WIFI_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub async fn connect(
    spawner: &Spawner,
    timer: impl EspWifiTimerSource + 'static,
    mut rng: impl esp_wifi::EspWifiRngSource + 'static,
    wifi: WIFI<'static>,
    (ssid, password): (&str, &str),
) -> Result<Stack<'static>, Error> {
    let seed = rng.next_u64();

    let init: &'static _ = WIFI_CONTROLLER.init(esp_wifi::init(timer, rng)?);

    let (mut controller, wifi_interfaces) = esp_wifi::wifi::new(init, wifi)?;
    let _ = controller.set_power_saving(esp_wifi::config::PowerSaveMode::None);

    let config = Config::dhcpv4(DhcpConfig::default());

    debug!("Initialize network stack");
    let stack_resources: &'static mut _ = STACK_RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(wifi_interfaces.sta, config, stack_resources, seed);

    spawner.must_spawn(connection(
        controller,
        ssid.try_into().unwrap(),
        password.try_into().unwrap(),
    ));
    spawner.must_spawn(net_task(runner));

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
async fn connection(controller: WifiController<'static>, ssid: String, password: String) {
    if let Err(error) = connection_fallible(controller, ssid, password).await {
        error!("Cannot connect to WiFi: {error:?}");
    }
}

async fn connection_fallible(
    mut controller: WifiController<'static>,
    ssid: String,
    password: String,
) -> Result<(), Error> {
    debug!("Start connection");
    debug!("Device capabilities: {:?}", controller.capabilities());
    let client_config = Configuration::Client(ClientConfiguration {
        ssid,
        password,
        //auth_method: AuthMethod::WPA2WPA3Personal,
        ..Default::default()
    });
    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            Timer::after(Duration::from_millis(5000)).await;
        }

        if !matches!(controller.is_started(), Ok(true)) {
            controller.set_configuration(&client_config)?;
            debug!("Starting WiFi controller");
            controller.start_async().await?;
            debug!("WiFi controller started");
        }

        debug!("Connect to WiFi network");

        match controller.connect_async().await {
            Ok(()) => {
                debug!("Connected to WiFi network");

                debug!("Wait for request to stop wifi");
                STOP_WIFI_SIGNAL.wait().await;
                info!("Received signal to stop wifi");
                controller.stop_async().await?;
                break;
            }
            Err(error) => {
                error!("Failed to connect to WiFi network: {error:?}");
                Timer::after(Duration::from_millis(5000)).await;
            }
        }
    }

    info!("Leave connection task");
    Ok(())
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[derive(Debug)]
pub enum Error {
    Setup,
    Operation,
}

impl From<WifiInitializationError> for Error {
    fn from(_: WifiInitializationError) -> Self {
        Self::Setup
    }
}

impl From<EspWifiError> for Error {
    fn from(_: EspWifiError) -> Self {
        Self::Operation
    }
}
