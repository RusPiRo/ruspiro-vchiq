/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Test Kernel
//!
//! Use this kernel and deploy to a Raspberry Pi 3B+ to verify the functions of the VCHIQ Interface
//!

#![no_std]
#![no_main]
#![feature(lang_items)]

#[macro_use]
extern crate ruspiro_boot;
extern crate ruspiro_allocator;

mod panic;

use ruspiro_brain::*;
use ruspiro_console::*;
use ruspiro_interrupt::*;
use ruspiro_mailbox::Mailbox;
use ruspiro_mmu as mmu;
use ruspiro_uart::*;
use ruspiro_vchiq as vchiq;

come_alive_with!(alive);
run_with!(run);

/// Entry point called from the bootstrapping code sequentielly for each core. At this point each core is running at
/// exeption level EL1.
///
/// This entry point allows to run further specifc initializations before any processing will start. When this
/// functions returns for a specific core this core will continue to call the [run] function and the next core will
/// enter this function.
pub fn alive(core: u32) {
    // use the mailbox property interface to request the ARM/GPU memory split to properly initialize the MMU
    // as the first activity initialize the MMU on every core to ensure any atomic operations would be valid
    let mut mailbox = Mailbox::new();
    let (vc_mem_start, vc_mem_size) = mailbox
        .get_vc_memory()
        .expect("Fatal issue getting VC memory split");

    // initialize the MMU
    unsafe { mmu::initialize(core, vc_mem_start, vc_mem_size) };

    // with the MMU in place other initialization is possible
    if core == 0 {
        let mut uart = Uart1::new();
        if uart.initialize(250_000_000, 115_200).is_ok() {
            CONSOLE.with_mut(|cons| cons.replace(uart));
        }

        // initialize the interrupt manager
        IRQ_MANAGER.with_mut(|irq| irq.initialize());
        enable_interrupts();

        info!("VCHIQ Testkernel is alive....");
        // initialize the brain
        BRAIN.lock().init();
        info!("Brain is initialized...");
        // spawn the initial thought to the brain
        spawn(test_vchiq());
    }
}

/// This is the main processing function every core will enter after the initialization has been done. This function is
/// intended to never return as the core does not have anything todo if this processing finishes. So the `run` function
/// represents the runtime of the OS on each core.
pub fn run(core: u32) -> ! {
    info!("Brain starts thinking on core {}", core);
    // once the next core has been kicked off we can start the Brain to pick up it's work and to enable async
    // functions beeing first class citicen in the NapuliOS
    // the *thinking* usually never returns, so the initial "main" thought should be spawned by this point already.
    BRAIN.read().do_thinking();

    loop {}
}

async fn test_vchiq() {
    info!("Run VCHIQ Test.....");
    let mut vchiq = vchiq::instance::VchiqInstance::new().unwrap();
    vchiq
        .connect()
        .await
        .map_err(|e| error!("{:?}", e))
        .unwrap();

    let params = vchiq::service::ServiceParams {
        fourcc: b"echo".into(),
        callback: None,
        version: 3,
        version_min: 3,
    };

    let service_handle = vchiq.open_service(params).await.unwrap();
    info!("Service {:?} opened", service_handle);

    vchiq.close_service(service_handle).await.unwrap();
    info!("Service closed");

    info!("Test finished");
}
