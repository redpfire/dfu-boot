use usb_device::{
    class_prelude::*,
    Result,
};

use core::mem;
use crate::flash;
use crate::flags;
use crate::config;
use crate::util;
use crate::util::LOGGER;

use stm32f1xx_hal::{
    pac::{FLASH, USART1},
    serial::Tx,
};

use core::marker::PhantomData;

#[allow(dead_code)]
pub(crate) const BL_MAGIC: u32 = 0xdeadcafe;

// const DFU_AL0: &'static str = "DFU Bootloader 0.2.0";
const CLASS_APPLICATION_SPECIFIC: u8 = 0xfe;
const SUBCLASS_DFU: u8 = 0x01;
const PROTOCOL_DFU_MODE: u8 = 0x02;
const DESC_DFU_FUNCTIONAL: u8 = 0x21;

#[allow(unused)]
pub(crate) mod dfu_request {
    pub const DFU_DETACH: u8 = 0; // proto 1
    pub const DFU_DNLOAD: u8 = 1; // proto 2
    pub const DFU_UPLOAD: u8 = 2; // proto 2
    pub const DFU_GETSTATUS: u8 = 3; // proto 1/2
    pub const DFU_CLRSTATUS: u8 = 4; // proto 2
    pub const DFU_GETSTATE: u8 = 5; // proto 1/2
    pub const DFU_ABORT: u8 = 6; // proto 2
}

#[allow(unused)]
#[derive(Copy, Clone)]
pub(crate) enum DfuState {
    AppIdle,
    AppDetach,

    DfuIdle,
    DfuDnloadSync,
    DfuDnloadBusy,
    DfuDnloadIdle,
    DfuManifestSync,
    DfuManifest,
    DfuManifestWaitReset,
    DfuUploadIdle,
    DfuError,
}

// todo: make all these useful
#[allow(unused)]
#[derive(Copy, Clone)]
pub(crate) enum DfuDeviceStatus {
    Ok, // No error condition is present.
    ErrTarget, // File is not targeted for use by this device.
    ErrFile, // File is for this device but fails some vendor-specific verification test.
    ErrWrite, // Device is unable to write memory.
    ErrErase, // Memory erase function failed.
    ErrCheckErased, // Memory erase check failed
    ErrProg, // Program memory function failed.
    ErrVerify, // Programmed memory failed verification.
    ErrAddress, // Cannot program memory due to received address that is out of range.
    ErrNotDone, // Received DFU_DNLOAD with wLength = 0, but device does not think it has all of the data yet.
    ErrFirmware, // Device’s firmware is corrupt.  It cannot return to run-time (non-DFU) operations.
    ErrVendor, // iString indicates a vendor-specific error.
    ErrUsbR, // Device detected unexpected USB reset signaling.
    ErrPoR, // Device detected unexpected power on reset.
    ErrUnknown, // Something went wrong, but the device does not know what it was.
    ErrStaledPkt, // Device stalled an unexpected request.
}

const BIGGEST_PAGE: usize = 2048;

pub struct Dfu<'a, B: UsbBus> {
    woosh: PhantomData<B>,
    comm_if: InterfaceNumber,
    strs: [StringIndex; config::ALT_SETTINGS],
    curr_alt: u8,
    upload_capable: bool,
    download_capable: bool,
    state: DfuState,
    status: DfuDeviceStatus,
    firmware_size: usize,
    awaits_flash: bool,
    flashing: bool,
    manifesting: bool,
    page_buffer: [u8; BIGGEST_PAGE],
    page_buffer_index: usize,
    flags: core::option::Option<&'a flags::BlFlags>,
}

impl<B: UsbBus> Dfu<'_, B> {
    pub fn new(alloc: &UsbBusAllocator<B>, download_capable: bool, tx: Option<Tx<USART1>>) -> Dfu<'_, B> {
        unsafe { LOGGER = tx }
        let flags = flags::read_bl_flags();
        let mut d = Dfu {
            woosh: PhantomData,
            comm_if: alloc.interface(),
            strs: unsafe { mem::zeroed() },
            curr_alt: 0,
            upload_capable: false,
            download_capable: download_capable,
            state: DfuState::DfuIdle,
            status: DfuDeviceStatus::Ok,
            firmware_size: 0,
            awaits_flash: false,
            flashing: false,
            manifesting: false,
            page_buffer: unsafe { mem::zeroed() },
            page_buffer_index: 0,
            flags: flags,
        };
        for i in 0..config::ALT_SETTINGS {
            d.strs[i] = alloc.string();
        }
        d
    }

    pub fn flags(&self) -> core::option::Option<&'_ flags::BlFlags> {
        self.flags
    }

    pub fn process_flash(&mut self) {
        if self.awaits_flash && !self.flashing {
            self.flashing = true;
            cortex_m::interrupt::free(|_| {
            unsafe {
                let mut addr: u32 = flash::PAGE_START +
                    self.firmware_size as u32;
                flash::erase_page(addr);
                let n: usize = (self.page_buffer_index) / 4;
                for i in 0..n {
                    let d: u32 = u32::from_le_bytes(
                        [self.page_buffer[i*4], self.page_buffer[(i*4)+1],
                        self.page_buffer[(i*4)+2], self.page_buffer[(i*4)+3]]);
                    match flash::write_word(addr, d) {
                        Ok(_) => {
                            self.status = DfuDeviceStatus::Ok;
                            addr += 4;
                        },
                        Err(_) => {
                            util::_log_fmt(format_args!("Write failed on i: {}  addr: 0x{:x} sr: 0x{:x}\r\n", i, addr, &(*(FLASH::ptr())).sr.read().bits()));
                            self.status = DfuDeviceStatus::ErrWrite;
                            
                            self.page_buffer_index = 0;
                            self.awaits_flash = false;
                            self.flashing = false;
                            return;
                        },
                    }
                }
                self.firmware_size += self.page_buffer_index;
                self.page_buffer_index = 0;
            }
            });
            self.awaits_flash = false;
            self.flashing = false;
        }
        else if self.manifesting && !self.flashing {
            self.flashing = true;
            let flash_count: u32 = match self.flags() {
                Some(flags) => flags.flash_count+1,
                None => 1,
            };
            let flags = &flags::BlFlags {
                magic: BL_MAGIC,
                flash_count: flash_count,
                user_code_legit: true,
                user_code_present: true,
                user_code_length: self.firmware_size as u32,
            };
            flags::write_bl_flags(flags);
            self.flags = flags::read_bl_flags();
            unsafe { flash::lock_flash(); }
            self.manifesting = false;
            self.flashing = false;
        }
    }
}

impl<B:UsbBus> UsbClass<B> for Dfu<'_, B> {
    fn get_bos_descriptors(&self, w: &mut BosWriter) -> Result<()> {
        w.capability(0x05, &[
            0x0,

            0xDF, 0x60, 0xDD, 0xD8, // MS OS 2.0 Platform Capability ID
            0x89, 0x45, 0xC7, 0x4C,
            0x9C, 0xD2, 0x65, 0x9D,
            0x9E, 0x64, 0x8A, 0x9F,

            0x0, 0x0, 0x03, 0x06, // Windows Version
            
            0xB2, 0x00, 0x21, 0x00
        ])
    }
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        for alt in 0..config::ALT_SETTINGS {
            writer.interface_alt(self.comm_if, alt as u8,
                                CLASS_APPLICATION_SPECIFIC,
                                SUBCLASS_DFU,
                                PROTOCOL_DFU_MODE,
                                Some(self.strs[alt]))?;
        }

        writer.write(DESC_DFU_FUNCTIONAL, &[
                     0x4 // manifestation tolerant
                        | if self.upload_capable { 0x02 } else { 0x00 }
                        | if self.download_capable { 0x01 } else { 0x00 }, //bmAttributes
                     255, 0, // wDetachTimeout
                     //(page_size & 0xff) as u8,
                     //((page_size >> 8) & 0xff) as u8, // wTransferSize
                     0x00, 0x01, // 256 bytes max
                     0x10, 0x01, // bcdDFUVersion
                     ])?;
        Ok(())
    }

    fn get_alt_setting(&mut self, interface: InterfaceNumber) -> Option<u8> {
        if interface == self.comm_if {
            Some(self.curr_alt)
        } else {
            None
        }
    }

    fn set_alt_setting(&mut self, interface: InterfaceNumber, alt: u8) -> bool {
        if interface == self.comm_if {
            self.curr_alt = alt;
            true
        }
        else {
            false
        }
    }

    fn get_string(&self, index: StringIndex, _lang_id: u16) -> Option<&str> {
        for i in 0..config::ALT_SETTINGS {
            if self.strs[i] == index {
                return Some(config::ALT_STRS[i]);
            }
        }
        None
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = *xfer.request();
        if !(req.request_type == control::RequestType::Class
             && req.recipient == control::Recipient::Interface
             && req.index == u8::from(self.comm_if) as u16) {
            return;
        }

        fn accept_status<B: UsbBus> (xfer: ControlIn<B>, c: &Dfu<B>, wait_time_ms: u32) {
            xfer.accept_with(&[
                             c.status as u8,
                             (wait_time_ms & 0xff) as u8,
                             ((wait_time_ms >> 8) & 0xff) as u8,
                             ((wait_time_ms >> 16) & 0xff) as u8,
                             c.state as u8,
                             0,
            ]).ok();
        }


        match self.status {
            DfuDeviceStatus::Ok => {},
            _ => {
                self.state = DfuState::DfuError;
            },
        }

        match self.state {
            DfuState::DfuDnloadBusy => {
                if !self.awaits_flash {
                    self.state = DfuState::DfuDnloadSync;
                }
            },
            _ => {},
        }

        match req.request {
            dfu_request::DFU_UPLOAD if req.value == 0
                && req.length > 0
                && self.upload_capable => {
            },
            dfu_request::DFU_GETSTATUS if req.value == 0
                && req.length == 6 => {
                    match self.state {
                        DfuState::DfuDnloadSync => {
                            if self.awaits_flash {
                                self.state = DfuState::DfuDnloadBusy;
                            }
                            else {
                                self.state = DfuState::DfuDnloadIdle;
                            }
                            self.status = DfuDeviceStatus::Ok;
                            accept_status(xfer, &self, 0);
                        },
                        DfuState::DfuDnloadBusy => {
                            self.state = DfuState::DfuDnloadSync;
                            accept_status(xfer, &self, 500);
                        },
                        DfuState::DfuManifest => {
                            if self.manifesting {
                                accept_status(xfer, &self, 500);
                            }
                            else {
                                self.state = DfuState::DfuManifestSync;
                                accept_status(xfer, &self, 0);
                            }
                        },
                        DfuState::DfuManifestSync => {
                            self.state = DfuState::DfuIdle;
                            accept_status(xfer, &self, 0);
                        },
                        _ => accept_status(xfer, &self, 0),
                    }
            },
            dfu_request::DFU_GETSTATE if req.value == 0
                && req.length == 1 => {
                    xfer.accept_with(&[ self.state as u8 ]).ok();
            },
            _ => {
                self.state = DfuState::DfuError;
                self.status = DfuDeviceStatus::ErrStaledPkt;
                util::_log_fmt(format_args!("Stalled pkt  req: {:?}\r\n", req));
                xfer.reject().ok();
            },
        }
    }

    fn control_out<'a>(&mut self, xfer: ControlOut<B>) {
        let req = *xfer.request();
        if !(req.request_type == control::RequestType::Class
             && req.recipient == control::Recipient::Interface
             && req.index == u8::from(self.comm_if) as u16) {
            return;
        }

        match req.request {
            dfu_request::DFU_DNLOAD if self.download_capable => {
                    if req.length > 0 {
                        match self.state {
                            DfuState::DfuIdle | DfuState::DfuDnloadIdle => {
                                unsafe{ flash::unlock_flash(); }
                                let start = self.page_buffer_index;
                                //let len = req.length as usize;
                                let len = xfer.data().len();
                                let page_size = unsafe { flash::get_flash_pg_size() };
                                self.page_buffer[start..start+len]
                                    .copy_from_slice(&xfer.data()[..len]);
                                self.page_buffer_index = start + len;
                                if self.page_buffer_index == page_size as usize {
                                    self.awaits_flash = true;
                                }
                                self.state = DfuState::DfuDnloadSync;
                                xfer.accept().ok();
                            },
                            _ => {xfer.reject().ok();},
                        }
                    }
                    else {
                        self.state = DfuState::DfuManifest;
                        self.manifesting = true;
                        self.awaits_flash = true;
                        xfer.accept().ok();
                    }
            },
            dfu_request::DFU_CLRSTATUS if req.value == 0
                && req.length == 0 => {
                    match self.state {
                        DfuState::DfuError => {
                            self.state = DfuState::DfuIdle;
                            self.status = DfuDeviceStatus::Ok;
                            xfer.accept().ok();
                        },
                        _ => {xfer.reject().ok();},
                    }
            },
            dfu_request::DFU_ABORT if req.value == 0
                && req.length == 0 => {
                    match self.state {
                        DfuState::DfuIdle => {
                            self.status = DfuDeviceStatus::Ok;
                            xfer.accept().ok();
                        },
                        _ => {xfer.reject().ok();}
                    }
            },
            _ => {
                self.state = DfuState::DfuError;
                self.status = DfuDeviceStatus::ErrStaledPkt;
                xfer.reject().ok();
            },
        }
    }
}
