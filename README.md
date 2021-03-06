# Bare-Metal VCHIQ interface

This highly work in progress crate provides access to the VideoCore features and connected devices utilizing the VCHIQ interface.

The crate comes with a test implementation to use the library. However, this can only be verified on actual Raspberry Pi hardware. QEMU does not emulate the VideoCore side to support the VCHQI interface.

## Current State

This crate is still in its very early stages and will hopefully be a complete bare metal implementation of the VCHIQ at some day. The VCHIQ implementation requires to execute specific parts of it concurrently. To do so - and without using threads - the Rust async executor/runtime `ruspiro-brain` is used. To run some tests and verify the functionality currently implemented this library crate also comes with a binary part that can be compiled into a `kernel8.img` to be put onto an actual Raspberry Pi 3(B+) for testing. This kernel uses the *miniUART* of the Raspberry Pi as the console output.

The following list shall give an overview of the actual implementation state and the planned function for the near future:

- [x] VCHIQ Instance and global state initialization
- [x] Initial Handshake from ARM to VC with the mailbox interface
- [x] exchange VCHIQ Connect message btw. ARM and VC
- [x] implement service creation and opening
- [ ] implement service closure
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
    Device acknowledged. Send kernel size 184640
    Device acknowledged. Send kernel...
    Kernel successfully sent
    mirroring
    new kernel received, preparing re-boot...
    re-boot in progress ... mode 64
    I: kernel - VCHIQ Testkernel is alive....
    I: kernel - Brain is initialized...
    I: kernel - Brain starts thinking on core 0
    I: kernel - Run VCHIQ Test.....
    I: ruspiro_vchiq::vchiq::instance - create the VCHIQ Instance
    I: ruspiro_vchiq::vchiq::state - SharedMemory: 0xffffffffc00ef000 / 0xc00ef000 - size 266240
    I: ruspiro_vchiq::vchiq::shared::slot - set message in slot 33, base ptr 0xffffffffc00ef000, slot start ptr 0xffffffffc0110000
    I: ruspiro_vchiq::vchiq::instance - try to connect VchiqInstance
    I: ruspiro_vchiq::vchiq::state - queue message CONNECT
    I: ruspiro_vchiq::vchiq::state - reserve space of 8 bytes from the slot  with 4096 bytes
    I: ruspiro_vchiq::vchiq::state - Space reserved at SlotPosition(
        34,
        0,
    ) with size 8
    I: ruspiro_vchiq::vchiq::state - slot locked
    I: ruspiro_vchiq::vchiq::state - quota locked
    I: ruspiro_vchiq::vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00ef000, slot start ptr 0xffffffffc0111000
    I: ruspiro_vchiq::vchiq::state - prepare message: CONNECT, pos: SlotPosition(34, 0), size: 0
    I: ruspiro_vchiq::vchiq::state - set local_tx_pos to 8
    I: ruspiro_vchiq::vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::vchiq::state - message queued and VC notified
    I: ruspiro_vchiq::vchiq::state - update connection state
    I: ruspiro_vchiq::vchiq::state - wait for VC CONNECT
    I: ruspiro_vchiq::vchiq::slothandler - slot_queue_index 0
    I: ruspiro_vchiq::vchiq::slothandler - slot_index 2
    I: ruspiro_vchiq::vchiq::slothandler - handle message from Some(SlotPosition(2, 0))
    I: ruspiro_vchiq::vchiq::slothandler - slot header - message type CONNECT / size 0x0
    I: ruspiro_vchiq::vchiq::slothandler - CONNECT - version common 8
    I: ruspiro_vchiq::vchiq::slothandler - new rx_pos 8
    I: ruspiro_vchiq::vchiq::state - add new service 'echo' at index 0
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' FREE -> OPENING
    I: ruspiro_vchiq::vchiq::state - queue message OPEN
    I: ruspiro_vchiq::vchiq::state - reserve space of 24 bytes from the slot  with 4088 bytes
    I: ruspiro_vchiq::vchiq::state - Space reserved at SlotPosition(
        34,
        8,
    ) with size 24
    I: ruspiro_vchiq::vchiq::state - slot locked
    I: ruspiro_vchiq::vchiq::state - quota locked
    I: ruspiro_vchiq::vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00ef000, slot start ptr 0xffffffffc0111000
    I: ruspiro_vchiq::vchiq::state - prepare message: OPEN, pos: SlotPosition(34, 8), size: 12
    I: ruspiro_vchiq::vchiq::state - copy message context
    I: ruspiro_vchiq::vchiq::state - set local_tx_pos to 32
    I: ruspiro_vchiq::vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::vchiq::state - message queued and VC notified
    I: ruspiro_vchiq::vchiq::state - waiting for OpenService ACK/NACK
    I: ruspiro_vchiq::vchiq::slothandler - handle message from Some(SlotPosition(2, 8))
    I: ruspiro_vchiq::vchiq::slothandler - slot header - message type OPENACK / size 0x2
    I: ruspiro_vchiq::vchiq::slothandler - Message: OpenAckPayload { version: 3 }
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' OPENING -> OPEN
    I: ruspiro_vchiq::vchiq::slothandler - new rx_pos 24
    I: ruspiro_vchiq::vchiq::state - Service state: OPEN
    I: ruspiro_vchiq::vchi::instance - queue message for service ServiceHandle(0), data TestParameter { magic: 4, blocksize: 0, iters: 100, verify: 1, echo: 1, align_size: 1, client_align: 0, server_align: 0, client_message_quota: 0, server_message_quota: 0 }
    I: ruspiro_vchiq::vchiq::state - queue message DATA
    I: ruspiro_vchiq::vchiq::state - reserve space of 48 bytes from the slot  with 4064 bytes
    I: ruspiro_vchiq::vchiq::state - Space reserved at SlotPosition(
        34,
        32,
    ) with size 48
    I: ruspiro_vchiq::vchiq::state - slot locked
    I: ruspiro_vchiq::vchiq::state - quota locked
    I: ruspiro_vchiq::vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00ef000, slot start ptr 0xffffffffc0111000
    I: ruspiro_vchiq::vchiq::state - prepare message: DATA, pos: SlotPosition(34, 32), size: 40
    I: ruspiro_vchiq::vchiq::state - copy message context
    I: ruspiro_vchiq::vchiq::state - set local_tx_pos to 80
    I: ruspiro_vchiq::vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::vchiq::state - message queued and VC notified
    I: kernel - wait until message has been processed
    I: ruspiro_vchiq::vchiq::slothandler - handle message from Some(SlotPosition(2, 24))
    I: ruspiro_vchiq::vchiq::slothandler - slot header - message type DATA / size 0x1
    I: ruspiro_vchiq::vchiq::slothandler - DATA Message local/remote 0/75 - srv.local/remote 0/75
    I: ruspiro_vchiq::vchiq::slothandler - data received for service, invoke callback with SlotMessage { header: SlotMessageHeader { msgid -> msg_type: DATA, msgid -> localport: 0, msgid -> remoteport: 75 }, data: [0] }
    I: ruspiro_vchiq::vchiq::instance - internal service callback called with reason MESSAGE_AVAILABLE
    I: ruspiro_vchiq::vchiq::instance - add completion
    I: ruspiro_vchiq::vchiq::instance::completion - completion added, insert event raised
    I: ruspiro_vchiq::vchiq::slothandler - new rx_pos 40
    I: ruspiro_vchiq::vchiq::instance - completion event received
    I: ruspiro_vchiq::vchi::completion - invoke vchi service callback,  reason: MESSAGE_AVAILABLE, message Some(SlotMessage { header: SlotMessageHeader { msgid -> msg_type: DATA, msgid -> localport: 0, msgid -> remoteport: 75 }, data: [0] })
    I: kernel - vchi callback invoked with reason MESSAGE_AVAILABLE, user_data: Some(UserData(DataLock { Value: Any, ReadLocks: 0 }))
    I: ruspiro_vchiq::vchi::instance - closing service
    I: ruspiro_vchiq::vchiq::state - closing service w. local port 0
    I: ruspiro_vchiq::vchiq::state - queue message CLOSE
    I: ruspiro_vchiq::vchiq::state - reserve space of 8 bytes from the slot  with 4016 bytes
    I: ruspiro_vchiq::vchiq::state - Space reserved at SlotPosition(
        34,
        80,
    ) with size 8
    I: ruspiro_vchiq::vchiq::state - slot locked
    I: ruspiro_vchiq::vchiq::state - quota locked
    I: ruspiro_vchiq::vchiq::shared::slot - set message in slot 34, base ptr 0xffffffffc00ef000, slot start ptr 0xffffffffc0111000
    I: ruspiro_vchiq::vchiq::state - prepare message: CLOSE, pos: SlotPosition(34, 80), size: 0
    I: ruspiro_vchiq::vchiq::state - set local_tx_pos to 88
    I: ruspiro_vchiq::vchiq::state - set state CLOSESENT
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' OPEN -> CLOSESENT
    I: ruspiro_vchiq::vchiq::doorbell - VC doorbell rung
    I: ruspiro_vchiq::vchiq::state - message queued and VC notified
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' CLOSESENT -> CLOSESENT
    I: ruspiro_vchiq::vchiq::slothandler - handle message from Some(SlotPosition(2, 40))
    I: ruspiro_vchiq::vchiq::slothandler - slot header - message type CLOSE / size 0x0
    I: ruspiro_vchiq::vchiq::slothandler - close service with state CLOSESENT
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' CLOSESENT -> CLOSED
    I: ruspiro_vchiq::vchiq::state - free service
    I: ruspiro_vchiq::vchiq::service - set new state for service 'echo' CLOSED -> FREE
    I: ruspiro_vchiq::vchiq::slothandler - new rx_pos 48
    I: ruspiro_vchiq::vchi::instance - vchiq service closed, now remove vchi side
    I: kernel - Test: Ok(())
    I: kernel - Test finished
    ```
