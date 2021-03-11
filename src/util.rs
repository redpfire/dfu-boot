use cortex_m::peripheral::{SCB, NVIC};
use core::fmt::Write;
use stm32f1xx_hal::{
    pac::{STK, RCC, USART1},
    serial::Tx,
};
use crate::flash;
use crate::flags;

pub(crate) static mut LOGGER: Option<Tx<USART1>> = None;

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

pub(crate) unsafe fn jump_to_usercode() {
    let scb = &*SCB::ptr();
    let nvic = &*NVIC::ptr();
    let stk = &*STK::ptr();
    let rcc = &*RCC::ptr();
    match flags::read_bl_flags() {
        Some(flags) => {
            if flags.user_code_present {
                cortex_m::interrupt::free(|_| {
                    _log_str("Jumping to User Code\r\n");
                    const STACK_POINTER: u32 = flash::PAGE_START;
                    const ENTRY_POINT: u32 = flash::PAGE_START+4;

                    let user_msp = core::ptr::read_volatile(STACK_POINTER as *const u32);
                    let user_jmp = core::ptr::read_volatile(ENTRY_POINT as *const u32);
                    let offset: u32 = flash::PAGE_START - 0x08000000;

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
