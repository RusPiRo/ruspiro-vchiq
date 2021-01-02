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
#![feature(future_poll_fn)]

extern crate alloc;
#[macro_use]
extern crate ruspiro_boot;
extern crate ruspiro_allocator;

mod panic;

use alloc::{boxed::Box, sync::Arc};
use core::{any::Any, future::poll_fn, task::Poll};
use ruspiro_brain::*;
use ruspiro_console::*;
use ruspiro_interrupt::*;
use ruspiro_lock::{RWLock, Semaphore};
use ruspiro_mailbox::Mailbox;
use ruspiro_mmu as mmu;
use ruspiro_uart::*;
use ruspiro_vchiq as vchi;
use vchi::{
    error::VchiResult,
    types::{FourCC, Reason, ServiceHandle, UserData},
};

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
    let mut instance = vchi::Instance::new();
    instance.initialize();
    instance
        .connect()
        .await
        .map_err(|e| error!("{:?}", e))
        .unwrap();

    info!(
        "Test: {:?}",
        vchiq_ping_test(&mut instance, b"echo".into()).await
    );

    //    test_vchiq_ping(&vchiq, b"echo".into()).await;

    info!("Test finished");
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct TestParameter {
    magic: i32,
    blocksize: i32,
    iters: i32,
    verify: i32,
    echo: i32,
    align_size: i32,
    client_align: i32,
    server_align: i32,
    client_message_quota: i32,
    server_message_quota: i32,
}

#[repr(i32)]
enum TestMessage {
    Error = 0,
    OneWay = 1,
    Async = 2,
    Sync = 3,
    Config = 4,
    Echo = 5,
}

#[derive(Debug)]
struct CallbackData {
    service: Option<ServiceHandle>,
    reply_event: Semaphore,
}

fn vchi_clnt_callback(user_data: Option<UserData>, reason: Reason /*void* handle */) {
    info!(
        "vchi callback invoked with reason {:?}, user_data: {:?}",
        reason, user_data
    );
    if let Some(user_data) = user_data {
        let user_data = user_data.read();
        let callback_data = user_data.downcast_ref::<CallbackData>().unwrap();
        match reason {
            Reason::MESSAGE_AVAILABLE => {
                // TODO: call vchi_instance.msg_dequeue() as long as it returns "ok"
                // if the response data length == 1 -> this was a "sync point"
                //
                callback_data.reply_event.up();
            }
            Reason::BULK_TRANSMIT_DONE => todo!(),
            Reason::BULK_RECEIVE_DONE => todo!(),
            Reason::BULK_TRANSMIT_ABORTED => info!("BULK_TARNSMIT_ABORTED"),
            Reason::BULK_RECEIVE_ABORTED => info!("BULK_RECEIVE_ABORTED"),
            _ => (),
        }
    } else {
        error!("vchi_callback called without user data");
    }
}

async fn vchiq_ping_test(instance: &mut vchi::Instance, fourcc: FourCC) -> VchiResult<()> {
    let callback_param = UserData::new(CallbackData {
        service: None,
        reply_event: Semaphore::new(0),
    });

    let setup = vchi::service::ServiceCreation {
        service_id: fourcc,
        version: 3,
        version_min: 3,
        callback: Some(Box::new(vchi_clnt_callback)),
        callback_param: Some(callback_param.clone()),
        rx_fifo_size: 0,
        tx_fifo_size: 0,
        want_unaligned_bulk_rx: false,
        want_unaligned_bulk_tx: false,
        want_crc: false,
    };

    let service = instance.service_open(setup).await?;
    callback_param
        .lock()
        .downcast_mut::<CallbackData>()
        .unwrap()
        .service
        .replace(service);
    let test_message = TestParameter {
        magic: TestMessage::Config as i32,
        blocksize: 0,
        iters: 100,
        verify: 1,
        echo: 1,
        align_size: 1,
        client_align: 0,
        server_align: 0,
        client_message_quota: 0,
        server_message_quota: 0,
    };

    instance.queue_message(service, test_message).await?;
    info!("wait until message has been processed");
    poll_fn(move |cx| {
        if callback_param
            .read()
            .downcast_ref::<CallbackData>()
            .unwrap()
            .reply_event
            .try_down()
            .is_ok()
        {
            Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;

    instance.service_close(service).await?;

    Ok(())
}
