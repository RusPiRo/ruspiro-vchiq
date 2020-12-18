/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Configuration Constants
//!
use super::shared::slotzero::SlotZero;
use core::mem;

/// Byte size of a Slot shared between ARM and VideoCore
pub const VCHIQ_SLOT_SIZE: usize = 4096;
/// Bit mask for Slot sizes
pub const VCHIQ_SLOT_MASK: usize = VCHIQ_SLOT_SIZE - 1;
/// Maximum number of Slots shared between ARM and VideoCore
pub const VCHIQ_MAX_SLOTS: usize = 128;
/// Number of Slots dedicated for each side (ARM and VC)
pub const VCHIQ_MAX_SLOTS_PER_SIDE: usize = 64;
/// Bit mask for slot queue
pub const VCHIQ_SLOT_QUEUE_MASK: usize = VCHIQ_MAX_SLOTS_PER_SIDE - 1;
/// maximum number of services we are able to handle
pub const VCHIQ_MAX_SERVICES: usize = 4096;
/// maximum number of bulk services we are able to handle
pub const VCHIQ_NUM_SERVICE_BULKS: usize = 4;
/// number of current bulks used
pub const VCHIQ_NUM_CURRENT_BULKS: usize = 32;
/// maximum number of fragments we are able to handle
pub const MAX_FRAGMENTS: usize = VCHIQ_NUM_CURRENT_BULKS * 2;

/// Number of Slots the [SlotZero] will occupy in the memory
pub const VCHIQ_SLOT_ZERO_SLOTS: usize =
    (mem::size_of::<SlotZero>() + VCHIQ_SLOT_SIZE - 1) / VCHIQ_SLOT_SIZE;
/// Total number of [Slot]s we are about to allocate for the interface
pub const TOTAL_SLOTS: usize = VCHIQ_SLOT_ZERO_SLOTS + 2 * 32;
