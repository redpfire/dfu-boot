
use crate::flash;
use crate::util;
use crate::dfu;
use stm32f1xx_hal::pac::FLASH;

#[allow(dead_code)]
const BL_FLAGS_HIGH: u32 = 0x0801fc00;
#[allow(dead_code)]
const BL_FLAGS_LOW: u32 = 0x0800fc00;

pub(crate) fn write_bl_flags(flags: &BlFlags) {
    unsafe fn _write(flags: &BlFlags, addr: u32) {
        util::_log_fmt(format_args!("Writing BL FLAGS to 0x{:x}\r\n", addr));
        let words: &[u32] = BlFlags::as_u32_slice(flags);
        // util::_log_fmt(format_args!("Slice: {:?}\r\n", words));
        for (pos, w) in words.iter().enumerate() {
            flash::write_word(addr+(pos as u32*4), *w).ok();
        }
    }
    unsafe {
        let flash = &*FLASH::ptr();
        flash::erase_page(BL_FLAGS_HIGH);
        let sr = flash.sr.read();
        // 128kb not supported, fall back to 64kb
        if sr.wrprterr().bit_is_set() || sr.pgerr().bit_is_set() || sr.eop().bit_is_clear() {
            util::_log_str("128 kb not supported\r\n");
            flash::erase_page(BL_FLAGS_LOW);
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
        if flags.magic != dfu::BL_MAGIC {
            util::_log_str("Magic in BL_FLAGS_HIGH not found\r\n");
            flags = &*(BL_FLAGS_LOW as *mut BlFlags);
            if flags.magic != dfu::BL_MAGIC {
                util::_log_str("Magic in BL_FLAGS_LOW not found\r\n");
                return None;
            }
            else {
                util::_log_fmt(format_args!("Flags from BL_FLAGS_LOW: {}\r\n", flags));
                return Some(flags);
            }
        }
        else {
            util::_log_fmt(format_args!("Flags from BL_FLAGS_HIGH: {}\r\n", flags));
            return Some(flags);
        }
    }
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
