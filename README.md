# dfu-boot
Device Firmware Upgrade (DFU) compatible bootloader for STM32F1xxx family of microcontrollers.

Could run on other STM32 devices, currently not tested.

### Features:
- DFU firmware download (to device) over USB
- WebUSB compatible
- Seamless boot into user code with an option to break into the bootloader at startup
- Flags stored in flash providing:
  - Authenticity of downloaded firmware
  - Magic value that prevents running user code if it's doesn't pass verification
  - Flash count
- Serial output for debugging over USART
- Based on Real-Time Interrupt-driven Concurrency (RTIC) framework for ARM Cortex-M microcontrollers

### Planned features:
- [ ] Cryptographic signature verification of downloaded firmware
- [ ] Firmware tamper detection
- [ ] Software or hardware assisted crypto
- [ ] CRC or SHA firmware integrity test
- [ ] DFU upload of existing firmware (to host)
