
use stm32f1xx_hal::pac::FLASH;

pub(crate) const FLASH_PAGESIZE: u32 = 0x1FFFF7E0;
pub(crate) const PAGE_START: u32 = 0x08004800;

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
