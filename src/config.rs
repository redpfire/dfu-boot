
use stm32f1xx_hal::{
    prelude::*,
    serial::Config,
};

pub(crate) const DEBUG: bool = true;

// USB constants
pub(crate) const USB_MANUFACTURER: &'static str = "aika";
pub(crate) const USB_PRODUCT: &'static str = concat!("DFU Bootloader ", env!("CARGO_PKG_VERSION"));
pub(crate) const USB_SERIAL_NO: &'static str = "8971842209015648";

pub(crate) const ALT_SETTINGS: usize = 2;
pub(crate) const ALT_STRS: &'static [&'static str] = &[concat!("DFU Bootloader ", env!("CARGO_PKG_VERSION")), "TEST"];

// URL which will pop up when DFU device is plugged in
pub(crate) const WEBUSB_URL: &'static str = "devanlai.github.io/webdfu/dfu-util";

pub(crate) fn usart1_config() -> Config {
    Config::default().baudrate(9_600.bps())
}
