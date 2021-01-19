#![no_std]
#![no_main]
#![feature(asm)]

use panic_halt as _;

extern crate stm32f1xx_hal as hal;
use rtic::app;

use cortex_m::{
    asm::{ wfi,delay }
    //peripheral::SCB,
};

use stm32f1xx_hal::{
    prelude::*,
    pac::{RCC,USB},
    pac,
    gpio::{ Floating, Input, Output, PushPull, gpioa::{PA11, PA12}, gpioc::PC13, State },
    timer::{Timer, CountDownTimer, Event},
    serial::{Serial, Config},
};
use embedded_hal::digital::v2::OutputPin;
use embedded_hal::digital::v2::InputPin;

use stm32_usbd::{ UsbBus, UsbPeripheral };
use usb_device::{
    prelude::*,
    bus,
};

use usbd_webusb::*;

mod dfu;

use crate::dfu::*;

const USB_PRODUCT: &'static str = concat!("DFU Bootloader ", env!("CARGO_PKG_VERSION"));

pub struct Peripheral {
    pub usb: USB,
    pub pin_dm: PA11<Input<Floating>>,
    pub pin_dp: PA12<Input<Floating>>,
}

unsafe impl Sync for Peripheral {}

unsafe impl UsbPeripheral for Peripheral {
    const REGISTERS: *const () = USB::ptr() as *const ();
    const DP_PULL_UP_FEATURE: bool = true;
    const EP_MEMORY: *const () = 0x4000_6000 as _;
    const EP_MEMORY_SIZE: usize = 512;

    fn enable() {
        let rcc = unsafe { &*RCC::ptr() };

        cortex_m::interrupt::free(|_| {
            // Enable USB peripheral
            rcc.apb1enr.modify(|_, w| w.usben().set_bit());

            // Reset USB peripheral
            rcc.apb1rstr.modify(|_, w| w.usbrst().set_bit());
            rcc.apb1rstr.modify(|_, w| w.usbrst().clear_bit());
        });
    }

    fn startup_delay() {
        // There is a chip specific startup delay. For STM32F103xx it's 1µs and this should wait for
        // at least that long.
        delay(72);
    }
}

pub type UsbBusType = UsbBus<Peripheral>;

pub(crate) fn check_sw_int() -> bool {
    unsafe {
        let rcc = &*RCC::ptr();
        if rcc.csr.read().sftrstf().bit_is_set() {
            return true;
        }
        else {
            return false;
        }
    }
}

#[app(device = stm32f1xx_hal::pac, peripherals = true)]
const APP: () = {
    struct Resources {
        USB_DEV: UsbDevice<'static, UsbBusType>,
        DFU: Dfu<'static, UsbBusType>,
        LED: PC13<Output<PushPull>>,
        BLINK: usize,
        TIMER_HANDLE: CountDownTimer<pac::TIM1>,
        #[init(false)]
        LED_STATE: bool,
        WUSB: WebUsb<UsbBusType>,
    }

    #[init]
    fn init(cx: init::Context) -> init::LateResources {
        static mut USB_BUS: Option<bus::UsbBusAllocator<UsbBusType>> = None;
        let device: pac::Peripherals = cx.device;

        let mut flash = device.FLASH.constrain();
        let mut rcc = device.RCC.constrain();

        let clocks = rcc.cfgr
            .use_hse(8.mhz())
            .sysclk(48.mhz())
            .pclk1(24.mhz())
            .freeze(&mut flash.acr);

        assert!(clocks.usbclk_valid());

        let mut gpioa = device.GPIOA.split(&mut rcc.apb2);
        let mut gpioc = device.GPIOC.split(&mut rcc.apb2);

        //if !(gpioc.pc14.is_high().ok().unwrap() || check_sw_int()) {
        //    // will fail if user code is not present or legit
        //    unsafe { jump_to_usercode(); }
        //}

        let mut usb_dp = gpioa.pa12.into_push_pull_output(&mut gpioa.crh);
        usb_dp.set_low().unwrap();
        delay(clocks.sysclk().0 / 100);

        let usb_dm = gpioa.pa11;
        let usb_dp = usb_dp.into_floating_input(&mut gpioa.crh);

        *USB_BUS = Some(UsbBus::new(Peripheral {
            pin_dp: usb_dp,
            pin_dm: usb_dm,
            usb: device.USB,
        }));

        let mut afio = device.AFIO.constrain(&mut rcc.apb2);

        let pin_tx = gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh);
        let pin_rx = gpioa.pa10;

        let serial = Serial::usart1(
            device.USART1,
            (pin_tx, pin_rx),
            &mut afio.mapr,
            Config::default().baudrate(9_600.bps()),
            clocks,
            &mut rcc.apb2,
        );

        let (tx, _) = serial.split();

        let dfu = Dfu::new(USB_BUS.as_ref().unwrap(), true, tx);
        if !(gpioc.pc14.is_high().ok().unwrap() || check_sw_int()) {
            // will fail if user code is not present or legit
            unsafe { jump_to_usercode(); }
            _log_str("User Code not present: Entering bootloader\r\n");
        }
        else {
            _log_str("Button pressed or Software Reboot: Entering bootloader\r\n");
        }

        let wusb = WebUsb::new(USB_BUS.as_ref().unwrap(), url_scheme::HTTPS,
                            "devanlai.github.io/webdfu/dfu-util");

        let mut blinks = 2;
        match dfu.flags() {
            Some(_) => {
                blinks = 4;
            },
            None => {}
        }

        let led = gpioc.pc13.into_push_pull_output_with_state(&mut gpioc.crh,
                                                                  State::High);
        
        let mut timer = Timer::tim1(device.TIM1, &clocks, &mut rcc.apb2).start_count_down(7.hz());
        timer.listen(Event::Update);

        let usb_dev =
            UsbDeviceBuilder::new(USB_BUS.as_ref().unwrap(), UsbVidPid(0x41ca, 0x2137))
            .manufacturer("aika")
            // .product("DFU Bootloader " + crate_version!())
            .product(USB_PRODUCT)
            .serial_number("8971842209015648")
            .max_packet_size_0(64)
            // .device_release(0x0200)
            .build();

        init::LateResources {
            USB_DEV: usb_dev,
            DFU: dfu,
            LED: led,
            BLINK: blinks,
            TIMER_HANDLE: timer,
            WUSB: wusb,
        }
    }

    #[idle]
    fn idle(_c: idle::Context) -> ! {
        loop {
            wfi();
        }
    }

    #[task(binds = TIM1_UP, priority = 1, resources = [LED, BLINK, LED_STATE, TIMER_HANDLE])]
    fn tim1_up(c: tim1_up::Context) {
        if *c.resources.BLINK % 2 == 0 {
            if *c.resources.LED_STATE {
                c.resources.LED.set_high().unwrap();
                *c.resources.LED_STATE = false;
            }
        } else {
                c.resources.LED.set_high().unwrap();
                c.resources.LED.set_low().unwrap();
                *c.resources.LED_STATE = true;
        }

        if *c.resources.BLINK > 0 {
            *c.resources.BLINK -= 1;
        }

        c.resources.TIMER_HANDLE.clear_update_interrupt_flag();
    }

    #[task(binds = USB_HP_CAN_TX, priority = 1, resources = [USB_DEV, DFU, WUSB])]
    fn USB_HP_CAN_TX(mut c: USB_HP_CAN_TX::Context) {
        usb_poll(&mut c.resources.USB_DEV, &mut c.resources.DFU, &mut c.resources.WUSB);
    }

    #[task(binds = USB_LP_CAN_RX0, priority = 1, resources = [USB_DEV, DFU, WUSB])]
    fn USB_LP_CAN_RX0(mut c: USB_LP_CAN_RX0::Context) {
        usb_poll(&mut c.resources.USB_DEV, &mut c.resources.DFU, &mut c.resources.WUSB);
    }
};

fn usb_poll<B: bus::UsbBus>(
    usb_dev: &mut UsbDevice<'static, B>,
    dfu: &mut Dfu<'static, B>,
    wusb: &mut WebUsb<B>,
) {
    if !usb_dev.poll(&mut [dfu, wusb]) {
        return;
    }
    dfu.process_flash();
}
