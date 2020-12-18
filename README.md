# Bare-Metal VCHIQ interface

This highly work in progress crate provides access to the VideoCore features and connected devices utilizing the VCHIQ interface.

The crate comes with a test implementation to use the library. However, this can only be verified on actual Raspberry Pi hardware. QEMU does not emulate the VideoCore side to support the VCHQI interface.

## Current State

This crate is still in its very early stages and will hopefully be a complete bare metal implementation of the VCHIQ at some day. The VCHIQ implementation requires to execute specific parts of it concurrently. To do so - and without using threads - the Rust async executor/runtime `ruspiro-brain` is used. To run some tests and verify the functionality currently implemented this library crate also comes with a binary part that can be compiled into a `kernel8.img` to be put onto an actual Raspberry Pi 3(B+) for testing. This kernel uses the *miniUART* of the Raspberry Pi as the console output.

The following list shall give an overview of the actual implementation state and the planned function for the near future:

- [x] VCHIQ Instance and global state initialization
- [x] Initial Handshake from ARM to VC with the mailbox interface
- [x] exchange VCHIQ Connect message btw. ARM and VC
- [ ] implement service creation and opening
- [ ] get usage of the *echo* service running and tested

## Build the Test Kernel

If you are new to bare metal Rust with the Raspberry Pi you might want to check out the [Tutorials](https://github.com/RusPiRo/ruspiro-tutorials) first to learn about the setup and the pipeline to successfully build your Raspberry Pi kernel.

> HINT: The current test kernel is prepared to build for the aarch64 target only!

For those just neednig a heads-up / quick start here are the steps in a nutshell:

> HINT: The command line calls should not be made from within the root folder of this repository as it contains specifc Rust build configuration that should not be considered during the installation and configuration of Rust!

1. Install Rust following the instructions from this [install page](https://www.rust-lang.org/tools/install)

2. Use the following commands to install the nightly toolchain and add the rust-src component both required for cross compilation:

    ```bash
    $> rustup install nightly
    $> rustup default nightly
    $> rustup component add rust-src
    $> rustup target add aarch64-unknown-none
    ```

3. Install the *aarch64-none-elf* cross compilation toolchain from [this page](https://developer.arm.com/tools-and-software/open-source-software/developer-tools/gnu-toolchain/gnu-a/downloads)

4. Install required *cargo* tools with the following commands:

    ```bash
    $> cargo install cargo-make
    $> cargo install cargo-xbuild
    ```

After those preparation steps the environment should be properly setup and the kernel could be build with the following command - executed from the projects root folder:

```bash
$> cargo make pi3
```

This will create the binary file `kernel8.img` in the subfolder `target` of this repository.

## Test VCHIQ

The first option to test the current implementation is to build the test kernel and put it on the SD card of the Raspberry Pi 3(B+). As this can become quite cumbersome it is recommended to install a bootloader to the Raspberry Pi SD card and use a tool to transfer new test kernel versions to it. To get this to work follow those steps:

1. Download the bootloader kernel and additional files that shall be present on the SD card for bare-metal from [here](https://github.com/RusPiRo/ruspiro-tutorials/tree/master/RPi)

2. Install an additional crago tool to push new built kernels to the RPi using a serial connector (USB-to-TTL) attached to the miniUART GPIO pins (TX0/RX0/GND) of the Raspberry Pi

    ```bash
    $> cargo install cargo-ruspiro-push
    ```

3. Build the kernel, connect the RPi and power it up and run the following command

    ```bash
    $> cargo make deploy
    ```

    > !HINT! The command assumes that the USB-TTL dongle maps to the serial port `COM3` on your PC. If you are a Mac/Linux user or the dongle is mapped to a different port the `Makefile.toml` in this repository need to be adjusted.

    ```toml
    [tasks.deploy]
        command = "cargo"
        args = ["ruspiro-push", "-k", "./target/kernel8.img", "-p", "COM3"]
        dependencies = [ "pi3" ]
    ```

    Change `COM3` to whatever serial port name. After deploying the new built kernel to the Raspberry Pi the actual console will display any console output from the kernel of the Raspberry Pi. To iterate over differemt tests just perform a power-off/on cycle on the Raspberry Pi and call the `cargo make deploy` command again.

4. in the current state the expected output will look like this:

    ```text
    Kernel file to send: ./target/kernel8.img with mode aarch64
    Send to port: COM3
    Send kernel to device. Initiate with token...
    Device acknowledged. Send kernel size 156800
    Device acknowledged. Send kernel...
    Kernel successfully sent
    mirroring
    new kernel received, preparing re-boot...
    re-boot in progress ... mode 64
    I: kernel - VCHIQ Testkernel is alive....
    I: kernel - Brain is initialized...
    I: kernel - Brain starts thinking on core 0
    I: kernel - Run VCHIQ Test.....
    I: ruspiro_vchiq::instance - create VchiqInstance
    I: ruspiro_vchiq::state - SharedMemory: 0xffffffffc00e8000 / 0xc00e8000 - size 266240
    I: ruspiro_vchiq::shared::slot - set message in slot 33, base ptr 0xffffffffc00e8000, slot start ptr 0xffffffffc0109000
    I: ruspiro_vchiq::instance - try to connect VchiqInstance
    I: ruspiro_vchiq::state - reserve space of 8 bytes from the slot  with 4096 bytes
    I: ruspiro_vchiq::state - Space reserved at SlotPosition(
        34,
        0,
    ) with size 8
    I: ruspiro_vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00e8000, slot start ptr 0xffffffffc010a000
    I: ruspiro_vchiq::state - prepare message: CONNECT, pos: SlotPosition(34, 0), size: 0
    I: ruspiro_vchiq::state - set local_tx_pos to 8
    I: ruspiro_vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::state - message queued and VC notified
    I: ruspiro_vchiq::state - update connection state
    I: ruspiro_vchiq::state - change conn_state from DISCONNECTED to CONNECTING
    I: ruspiro_vchiq::state - next stage
    I: ruspiro_vchiq::state - VCHIQ in 'connecting' state
    I: ruspiro_vchiq::state - slot_queue_index 0
    I: ruspiro_vchiq::state - slot_index 2
    I: ruspiro_vchiq::state - handle message from Some(SlotPosition(2, 0))
    I: ruspiro_vchiq::state - slot header - message type CONNECT / size 0x0
    I: ruspiro_vchiq::state - CONNECT - version common 8
    I: ruspiro_vchiq::state - new rx_pos 8
    I: ruspiro_vchiq::slothandler - wake slothandler by ref
    I: ruspiro_vchiq::state - connect semaphore down
    I: ruspiro_vchiq::state - change conn_state from CONNECTING to CONNECTED
    I: ruspiro_vchiq::state - add new service 'echo' at index 0
    I: ruspiro_vchiq::service - set new state for service 'echo' FREE -> OPENING
    I: ruspiro_vchiq::state - reserve space of 24 bytes from the slot  with 4088 bytes
    I: ruspiro_vchiq::state - Space reserved at SlotPosition(
        34,
        8,
    ) with size 24
    I: ruspiro_vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00e8000, slot start ptr 0xffffffffc010a000
    I: ruspiro_vchiq::state - prepare message: OPEN, pos: SlotPosition(34, 8), size: 12
    I: ruspiro_vchiq::state - copy message context
    I: ruspiro_vchiq::state - set local_tx_pos to 32
    I: ruspiro_vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::state - message queued and VC notified
    I: ruspiro_vchiq::state - waiting for OpenService ACK/NACK
    I: ruspiro_vchiq::state - handle message from Some(SlotPosition(2, 8))
    I: ruspiro_vchiq::state - slot header - message type OPENACK / size 0x2
    I: ruspiro_vchiq::state - Message: OpenAckPayload { version: 3 }
    I: ruspiro_vchiq::service - set new state for service 'echo' OPENING -> OPEN
    I: ruspiro_vchiq::state - new rx_pos 24
    I: ruspiro_vchiq::slothandler - wake slothandler by ref
    I: ruspiro_vchiq::state - Service state: OPEN
    I: kernel - Service ServiceHandle(4096) opened
    I: kernel - Test finished
    I: ruspiro_vchiq::instance - dropping VCHIQ
    ```