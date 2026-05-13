# ESP32-C6 TRMNL Firmware

Custom Rust firmware for the [TRMNL](https://usetrmnl.com) e-paper display platform, built for the **ESP32-C6** RISC-V microcontroller. It drives a Waveshare 7.5" V2 e-paper display, connects over Wi-Fi, fetches images from a TRMNL-compatible server, and renders them using the [QOI](https://qoiformat.org/) image format.

---

## Features

- **Async runtime** — built on [Embassy](https://embassy.dev/) with `esp-rtos`
- **Wi-Fi station** — automatic connection with DHCP, DNS, and TCP/TLS via `esp-radio`
- **E-paper display** — Waveshare **7.5" V2** (800×480px) with full refresh, powered on demand to save energy
- **QOI image decoding** — fast, embedded-friendly image format (`tinyqoi`)
- **Status LED** — WS2812B RGB LED on GPIO8 for visual feedback (boot, working, sleep, error)
- **Robust retry logic** — exponential backoff (15 s → 5 min) on network or display failures
- **Compile-time config** — Wi-Fi credentials and TRMNL endpoint baked in at build time via `embedded-config`

---

## Hardware Requirements

| Component | Spec | Notes |
|-----------|------|-------|
| MCU | ESP32-C6 DevKit | RISC-V, 160 MHz, 512 KB SRAM |
| Display | Waveshare 7.5" V2 e-paper | 800×480, SPI interface |
| Status LED | WS2812B / NeoPixel | Single RGB LED on GPIO8 |
| Power | USB-C or 5 V → 3.3 V reg | DevKit has onboard regulator |

> **Note:** The firmware powers the display via **GPIO10** driving a high-side switch (e.g., P-channel MOSFET). If your display HAT does not have a dedicated power-switch pin, wire GPIO10 to the gate of a MOSFET that switches 3.3 V to the display.

---

## Wiring Diagram

Connect the Waveshare 7.5" V2 HAT (or bare panel with driver board) to the ESP32-C6 DevKit as shown below.

```text
┌─────────────────────────────────────────────────────────────┐
│                    ESP32-C6 DevKit                          │
│                                                             │
│   3V3 ●──────────────────────┬───────────────────────┐      │
│   GND ●──────────────────────┼───────────────┐       │      │
│   GPIO19 (SCK)  ●────────────┼─────── CLK    │       │      │
│   GPIO20 (MOSI) ●────────────┼─────── DIN    │       │      │
│   GPIO18 (CS)   ●────────────┼─────── CS     │       │      │
│   GPIO21 (DC)   ●────────────┼─────── DC     │       │      │
│   GPIO22 (RST)  ●────────────┼─────── RST    │       │      │
│   GPIO23 (BUSY) ●────────────┼─────── BUSY   │       │      │
│   GPIO10 (PWR)  ●────────────┘       │       │       │      │
│                                      │       │       │      │
│   GPIO8 (WS2812) ●──[Status LED]─────┘       │       │      │
│                                              │       │      │
└──────────────────────────────────────────────┘       │      │
                                                       │      │
                              ┌────────────────────────┘      │
                              │    Waveshare 7.5" V2          │
                              │    E-Paper Driver HAT         │
                              │                               │
                              │   VCC  ●←─────────────────────┘
                              │   GND  ●←─────────────────────┘
                              │   DIN  ●←── GPIO20             │
                              │   CLK  ●←── GPIO19             │
                              │   CS   ●←── GPIO18             │
                              │   DC   ●←── GPIO21             │
                              │   RST  ●←── GPIO22             │
                              │   BUSY ●──→ GPIO23             │
                              │                               │
                              │  (Optional PWR control)       │
                              │        ●←── GPIO10  (MOSFET)   │
                              └───────────────────────────────┘
```

### Mermaid Diagram

```mermaid
flowchart LR
    subgraph ESP["ESP32-C6 DevKit"]
        GND1[GND]
        VCC1[3V3]
        SCK[GPIO19]
        MOSI[GPIO20]
        CS[GPIO18]
        DC[GPIO21]
        RST[GPIO22]
        BUSY[GPIO23]
        PWR[GPIO10]
        LED[GPIO8]
    end

    subgraph DISP["Waveshare 7.5\" V2 HAT"]
        GND2[GND]
        VCC2[VCC]
        D_CLK[CLK]
        D_DIN[DIN]
        D_CS[CS]
        D_DC[DC]
        D_RST[RST]
        D_BUSY[BUSY]
    end

    subgraph PERIPH["Peripherals"]
        WS["WS2812B LED"]
        MOSFET["P-Ch MOSFET<br/>(high-side switch)"]
    end

    SCK  --> D_CLK
    MOSI --> D_DIN
    CS   --> D_CS
    DC   --> D_DC
    RST  --> D_RST
    D_BUSY --> BUSY

    VCC1 --> MOSFET
    PWR  --> MOSFET
    MOSFET --> VCC2

    GND1 --- GND2
    GND1 --- WS

    LED --> WS
```

### Pin Mapping

| Function | ESP32-C6 GPIO | E-Paper HAT | Direction |
|----------|---------------|-------------|-----------|
| SPI SCK  | GPIO19        | CLK         | OUT       |
| SPI MOSI | GPIO20        | DIN         | OUT       |
| SPI CS   | GPIO18        | CS          | OUT       |
| Data/Command | GPIO21    | DC          | OUT       |
| Reset    | GPIO22        | RST         | OUT       |
| Busy     | GPIO23        | BUSY        | IN (PU)   |
| Display Power | GPIO10   | *(switch VCC)* | OUT   |
| Status LED | GPIO8      | WS2812B DIN | OUT       |

> **Pull-up:** `BUSY` is configured with an internal pull-up.

---

## Project Structure

```
.
├── Cargo.toml           # Rust dependencies & profiles
├── build_cfg.toml       # Wi-Fi & TRMNL credentials (compile-time config)
├── rust-toolchain.toml  # Nightly Rust + riscv32imac target
├── devenv.nix           # Nix/devenv shell with ESP tooling
├── .cargo/config.toml   # espflash runner & build flags
└── src/
    ├── main.rs          # Entry point, peripheral init, boot flow
    ├── wifi.rs          # Wi-Fi station connection task
    ├── http.rs          # HTTP/TLS client: fetch metadata + QOI images
    ├── epaper.rs        # E-paper driver (Waveshare 7.5" V2)
    └── status.rs        # WS2812B RGB LED status indicator
```

---

## Configuration

Edit `build_cfg.toml` before building:

```toml
[wifi]
ssid = "YOUR_SSID"
password = "YOUR_PASSWORD"

[trmnl]
address = "https://your-trmnl-instance.com"
device_id = "YOUR_DEVICE_UUID"
```

These values are embedded into the binary at compile time via `embedded-config`.

---

## Build & Flash

### Prerequisites

The repository uses [devenv](https://devenv.sh/) to provide a reproducible shell with all tools:

- `rustc` (nightly)
- `cargo`
- `espflash`
- `flip-link`

Enter the shell:

```bash
direnv allow   # or: devenv shell
```

### Build

```bash
cargo build --release
```

### Flash & Monitor

```bash
cargo run --release
```

This invokes `espflash flash --monitor --chip esp32c6`. The `--monitor` flag opens a serial terminal after flashing so you can view logs.

---

## Runtime Behavior

1. **Boot** — initializes Wi-Fi, SPI, and the e-paper display.
2. **API Request** — `GET <trmnl.address>/api/display` with header `Access-Token: <device_id>`.
3. **Parse JSON** — extract `image_url` and `refresh_rate`.
4. **Fetch Image** — `GET <image_url>` with `Accept: image/qoi`.
5. **Decode & Draw** — decode QOI → framebuffer → full e-paper refresh.
6. **Sleep** — deep-sleep loop for `refresh_rate` seconds, then repeat from step 2.

### Error Handling

- Any failure triggers an **exponential backoff** retry: 15 s → 30 s → 60 s → ... → max 300 s.
- Status LED colors:
  - 🟡 **Gold** — booting
  - 🟢 **Green** — working / fetching
  - 🔵 **Blue** — sleeping
  - 🔴 **Red** — runtime failure
  - 🟥 **Crimson** — boot failure (will reset in 3 s)

---

## Tech Stack

| Crate | Purpose |
|-------|---------|
| `esp-hal` | ESP32-C6 HAL (GPIO, SPI, RMT, timers) |
| `esp-radio` | Wi-Fi driver (`esp-radio` / ESP-IDF PHY) |
| `esp-rtos` | Embassy executor + ESP glue |
| `embassy-net` | Async TCP/IP stack with DHCP/DNS/TLS |
| `reqwless` | Async HTTP/TLS client |
| `epd-waveshare` | Waveshare e-paper driver |
| `embedded-graphics` | 2D drawing primitives |
| `tinyqoi` | QOI image decoder |
| `serde-json-core` | Heapless JSON deserialization |
| `esp-hal-smartled2` | WS2812B RMT driver |

---

## License

MIT or Apache-2.0, at your option.
