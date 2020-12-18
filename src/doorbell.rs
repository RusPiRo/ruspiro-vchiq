/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Doorbell
//!

use ruspiro_mmio_register::{define_mmio_register, ReadWrite};
use ruspiro_console::*;

/// Ring the VideoCore doorbell to let it know something has happened and it is time to pick up
/// processing
pub fn ring_vc_doorbell() {
    //info!("ring VC doorbell");
    BELL2::Register.set(0);
    info!("VC doorbell rung");
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
