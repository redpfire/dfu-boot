use usb_device::{
    class_prelude::*,
    Result,
};

use core::mem;

use stm32f1xx_hal::{
    pac::{FLASH, USART1, STK, RCC},
    serial::Tx,
};

use cortex_m::peripheral::{SCB, NVIC};

use core::marker::PhantomData;

// const DFU_AL0: &'static str = "DFU Bootloader 0.2.0";
const DFU_AL0: &'static str = concat!("DFU Bootloader ", env!("CARGO_PKG_VERSION"));
const CLASS_APPLICATION_SPECIFIC: u8 = 0xfe;
const SUBCLASS_DFU: u8 = 0x01;
const PROTOCOL_DFU_MODE: u8 = 0x02;
const DESC_DFU_FUNCTIONAL: u8 = 0x21;

const PAGE_START: u32 = 0x08004800;

#[allow(dead_code)]
const BL_FLAGS_HIGH: u32 = 0x0801fc00;
#[allow(dead_code)]
const BL_FLAGS_LOW: u32 = 0x0800fc00;
#[allow(dead_code)]
const BL_MAGIC: u32 = 0xdeadcafe;

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

static mut LOGGER: Option<Tx<USART1>> = None;
pub(crate) fn _log_str(s: &str) {
    unsafe {
        if LOGGER.is_some() {
            LOGGER.as_mut().unwrap().write_str(s).unwrap();
        }
    }
}
pub(crate) fn _log_fmt(args: core::fmt::Arguments) {
    unsafe {
        if LOGGER.is_some() {
            LOGGER.as_mut().unwrap().write_fmt(args).unwrap();
        }
    }
}
use core::fmt::Write;

pub(crate) fn write_bl_flags(flags: &BlFlags) {
    unsafe fn _write(flags: &BlFlags, addr: u32) {
        _log_fmt(format_args!("Writing BL FLAGS to 0x{:x}\r\n", addr));
        let words: &[u32] = BlFlags::as_u32_slice(flags);
        _log_fmt(format_args!("Slice: {:?}\r\n", words));
        for (pos, w) in words.iter().enumerate() {
            write_word(addr+(pos as u32*4), *w).ok();
        }
    }
    unsafe {
        let flash = &*FLASH::ptr();
        erase_page(BL_FLAGS_HIGH);
        let sr = flash.sr.read();
        // 128kb not supported, fall back to 64kb
        if sr.wrprterr().bit_is_set() || sr.pgerr().bit_is_set() || sr.eop().bit_is_clear() {
            _log_str("128 kb not supported\r\n");
            erase_page(BL_FLAGS_LOW);
            _write(flags, BL_FLAGS_LOW);
        }
        else {
            _write(flags, BL_FLAGS_HIGH);
        }
    }
}

pub(crate) fn read_bl_flags() -> core::option::Option<&'static BlFlags> {
    unsafe {
        let mut flags = &*(BL_FLAGS_HIGH as *mut BlFlags);
        if flags.magic != BL_MAGIC {
            _log_str("Magic in BL_FLAGS_HIGH not found\r\n");
            flags = &*(BL_FLAGS_LOW as *mut BlFlags);
            if flags.magic != BL_MAGIC {
                _log_str("Magic in BL_FLAGS_LOW not found\r\n");
                return None;
            }
            else {
                _log_fmt(format_args!("Flags from BL_FLAGS_LOW: {}\r\n", flags));
                return Some(flags);
            }
        }
        else {
            _log_fmt(format_args!("Flags from BL_FLAGS_HIGH: {}\r\n", flags));
            return Some(flags);
        }
    }
}

pub(crate) unsafe fn jump_to_usercode() {
    let scb = &*SCB::ptr();
    let nvic = &*NVIC::ptr();
    let stk = &*STK::ptr();
    let rcc = &*RCC::ptr();
    match read_bl_flags() {
        Some(flags) => {
            if flags.user_code_present {
                cortex_m::interrupt::free(|_| {
                    _log_str("Jumping to User Code\r\n");
                    const STACK_POINTER: u32 = PAGE_START;
                    const ENTRY_POINT: u32 = PAGE_START+4;

                    let user_msp = core::ptr::read_volatile(STACK_POINTER as *const u32);
                    let user_jmp = core::ptr::read_volatile(ENTRY_POINT as *const u32);
                    let offset: u32 = PAGE_START - 0x08000000;

                    //disable interrupts
                    nvic.icer[0].write(0xffffffff);
                    nvic.icer[1].write(0xffffffff);
                    nvic.icpr[0].write(0xffffffff);
                    nvic.icpr[1].write(0xffffffff);
                    //disable systick
                    stk.ctrl.modify(|_, w| w.enable().bit(false));
                    //reset clocks
                    rcc.cr.modify(|_, w| w.hsion().bit(true));
                    rcc.cfgr.modify(|r, w| w.bits(r.bits() & 0xf8ff0000));
                    rcc.cr.modify(|r, w| w.bits(r.bits() & 0xfef6ffff));
                    rcc.cr.modify(|r, w| w.bits(r.bits() & 0xfffbffff));
                    rcc.cfgr.modify(|r, w| w.bits(r.bits() & 0xff80ffff));
                    rcc.cir.write(|w| w.bits(0));

                    // after this jump it should not return
                    scb.vtor.write(offset);
                    cortex_m::register::msp::write(user_msp);
                    // asm!("bx $0" :: "r" (user_jmp) ::);
                    asm!("bx {}", in(reg) user_jmp);
                });
            }
        },
        None => {},
    }
}

pub(crate) const FLASH_PAGESIZE: u32 = 0x1FFFF7E0;
pub(crate) unsafe fn get_flash_pg_size() -> u16 {
    let r = core::ptr::read_volatile(FLASH_PAGESIZE as *const u32) & 0xffff;
    if r > 128 {
        return 0x800;
    }
    else {
        return 0x400;
    }
}

pub(crate) unsafe fn unlock_flash() {
    let flash = &*FLASH::ptr();

    flash.keyr.write(|w| w.bits(0x45670123));
    flash.keyr.write(|w| w.bits(0xCDEF89AB));
}

pub(crate) unsafe fn lock_flash() {
    let flash = &*FLASH::ptr();
    
    flash.cr.modify(|_, w| w.lock().bit(true));
}

pub(crate) unsafe fn write_word(addr: u32, data: u32) -> core::result::Result<(), ()> {
    let flash = &*FLASH::ptr();
    let a_32 = addr as *mut u32;
    let a: *mut u16 = a_32.cast();

    let lhw = (data & 0x0000ffff) as u16;
    let hhw = ((data & 0xffff0000) >> 16) as u16;

    for _ in 0..3 {
        while flash.sr.read().bsy().bit_is_set() {}
        flash.cr.modify(|_, w| w.strt().bit(false));
        flash.cr.modify(|_, w| w.pg().bit(true));
        core::ptr::write_volatile(a.offset(1), hhw);
        while flash.sr.read().bsy().bit_is_set() {}

        core::ptr::write_volatile(a, lhw);
        while flash.sr.read().bsy().bit_is_set() {}

        let read = core::ptr::read_volatile(addr as *mut u32);
        if read == data {
            flash.cr.modify(|_, w| w.pg().bit(false).strt().bit(false));
            while flash.sr.read().bsy().bit_is_set() {}
            return Ok(());
        }
    }

     Err(())
}

pub(crate) unsafe fn erase_page(addr: u32) {
    let flash = &*FLASH::ptr();

    while flash.sr.read().bsy().bit_is_set() {}
    flash.cr.modify(|_,w| w.per().bit(true));
    while flash.sr.read().bsy().bit_is_set() {}
    flash.ar.write(|w| w.far().bits(addr));
    flash.cr.modify(|_,w| w.strt().bit(true).per().bit(true));
    while flash.sr.read().bsy().bit_is_set() {}
    flash.cr.modify(|_, w| w.per().bit(false).strt().bit(false));
    while flash.sr.read().bsy().bit_is_set() {}
    flash.cr.write(|w| w.bits(0));
}

#[derive(Debug)]
pub struct BlFlags {
    pub magic: u32,
    pub flash_count: u32,
    pub user_code_legit: bool,
    pub user_code_present: bool,
    pub user_code_length: u32,
}

impl BlFlags {
    pub(crate) unsafe fn as_u32_slice<T: Sized>(p: &T) -> &[u32] {
        ::core::slice::from_raw_parts(
            (p as *const T) as *const u32,
            ::core::mem::size_of::<T>(),
        )
    }
}

impl core::fmt::Display for BlFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "BlFlags {{\r\n  MAGIC: 0x{:x}\r\n  Flash Count: {}\r\n  UserCode Legit: {}\r\n  UserCode Present: {}\r\n  UserCode Length: {}\r\n}}", self.magic, self.flash_count, self.user_code_legit, self.user_code_present, self.user_code_length)
    }
}


const BIGGEST_PAGE: usize = 2048;

pub struct Dfu<'a, B: UsbBus> {
    woosh: PhantomData<B>,
    comm_if: InterfaceNumber,
    def_str: StringIndex,
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
    flags: core::option::Option<&'a BlFlags>,
}

impl<B: UsbBus> Dfu<'_, B> {
    pub fn new(alloc: &UsbBusAllocator<B>, download_capable: bool, tx: Tx<USART1>) -> Dfu<'_, B> {
        unsafe { LOGGER = Some(tx); }
        let flags = read_bl_flags();
        Dfu {
            woosh: PhantomData,
            comm_if: alloc.interface(),
            def_str: alloc.string(),
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
        }
    }

    pub fn flags(&self) -> core::option::Option<&'_ BlFlags> {
        self.flags
    }

    pub fn process_flash(&mut self) {
        if self.awaits_flash && !self.flashing {
            self.flashing = true;
            cortex_m::interrupt::free(|_| {
            unsafe {
                let mut addr: u32 = PAGE_START +
                    self.firmware_size as u32;
                erase_page(addr);
                let n: usize = (self.page_buffer_index) / 4;
                for i in 0..n {
                    let d: u32 = u32::from_le_bytes(
                        [self.page_buffer[i*4], self.page_buffer[(i*4)+1],
                        self.page_buffer[(i*4)+2], self.page_buffer[(i*4)+3]]);
                    match write_word(addr, d) {
                        Ok(_) => {
                            self.status = DfuDeviceStatus::Ok;
                            addr += 4;
                        },
                        Err(_) => {
                            _log_fmt(format_args!("Write failed on i: {}  addr: 0x{:x} sr: 0x{:x}\r\n", i, addr, &(*(FLASH::ptr())).sr.read().bits()));
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
            let flags = &BlFlags {
                magic: BL_MAGIC,
                flash_count: flash_count,
                user_code_legit: true,
                user_code_present: true,
                user_code_length: self.firmware_size as u32,
            };
            write_bl_flags(flags);
            self.flags = read_bl_flags();
            unsafe { lock_flash(); }
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
        writer.interface_ex(self.comm_if,
                            CLASS_APPLICATION_SPECIFIC,
                            SUBCLASS_DFU,
                            PROTOCOL_DFU_MODE,
                            0, self.def_str.into())?;

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

    fn get_string(&self, index: StringIndex, _lang_id: u16) -> Option<&str> {
        if index == self.def_str {
            Some(DFU_AL0)
        } else {
            None
        }
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
                _log_fmt(format_args!("Stalled pkt  req: {:?}\r\n", req));
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
                                unsafe{ unlock_flash(); }
                                let start = self.page_buffer_index;
                                //let len = req.length as usize;
                                let len = xfer.data().len();
                                let page_size = unsafe { get_flash_pg_size() };
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
