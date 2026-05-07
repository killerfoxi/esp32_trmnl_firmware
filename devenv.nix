{ pkgs, ... }:

{
  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  packages = with pkgs; [
    cargo-edit
    espflash
    flip-link
    libusb1
  ];

  enterShell = ''
    echo "ESP32-C6 toolchain: $(rustc --version)"
    echo "Flash : cargo run"
    echo "Build : cargo build --release"
  '';
}
