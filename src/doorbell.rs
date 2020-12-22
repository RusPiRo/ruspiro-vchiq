/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Doorbell
//!

use alloc::sync::Arc;
use ruspiro_console::*;
use ruspiro_interrupt::*;
use ruspiro_lock::r#async::AsyncMutex;
use ruspiro_mmio_register::{define_mmio_register, ReadWrite};

//static mut STATE: *const Arc<AsyncMutex<StateInner>> = core::ptr::null();

/// Activate the interrut for the VC Doorbell
pub(crate) fn activate_doorbell() {
    //local: *const Arc<AsyncMutex<State>>) {
    IRQ_MANAGER.with_mut(|irq| irq.activate(Interrupt::ArmDoorbell0));
    info!("activated doorbell IRQ");
}

/// Ring the VideoCore doorbell to let it know something has happened and it is time to pick up
/// processing
pub(crate) fn ring_vc_doorbell() {
    //info!("ring VC doorbell");
    BELL2::Register.set(0);
    info!("VC doorbell rung");
}

#[IrqHandler(ArmDoorbell0)]
fn vchiq_doorbell_handler() {
    info!("vchiq doorbell irq");
    // if the VideoCore has rung the doorbell this indicates that data has changed and my require
    // the attention of the ARM side
    // so read the doorbell status
    let status = BELL0::Register.get();
    info!("doorbell value {}", status);
    if (status & 0x04) != 0 {
        info!("it's for me :)");
        //unsafe { info!("Local State {:#?}", *STATE) };
        // the desired doorbell was really rung - so poll the events for updates
        // this will trigger the event related semaphores to be raised which will then
        // be picked up by the corresponding futures
        // this might later just wake the futures as they might be able to make progress now
        //remote_event_pollall();
    }
}

/// RPi3 peripheral base address
const PERIPHERAL_BASE: usize = 0x3F00_0000;
/// Mailbox MMIO base address
const DOORBELL_BASE: usize = PERIPHERAL_BASE + 0x0000_B800;

define_mmio_register! [
    /// The doorbell rung by the VideoCore if there is something to do on ARM side
    BELL0<ReadWrite<u32>@(DOORBELL_BASE + 0x40)>,
    BELL1<ReadWrite<u32>@(DOORBELL_BASE + 0x44)>,
    BELL2<ReadWrite<u32>@(DOORBELL_BASE + 0x48)>,
    BELL3<ReadWrite<u32>@(DOORBELL_BASE + 0x4c)>
];
