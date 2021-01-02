/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SlotInfo
//!
//!

use core::fmt;

/// The [SLotInfo] stores the usage and release counters of each individual [Slot] that is used by either side (ARM/VC)
/// to transfer data between the two. As this data is shared between ARM and VideCoCore it can only be accessed with
/// it's accessor.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct SlotInfo {
    pub use_count: u16,
    pub release_count: u16,
}

pub struct SlotInfoAccessor {
    inner: *mut SlotInfo,
    len: usize,
}

impl SlotInfoAccessor {
    /// Create a new accessor for the [SlotInfo] data. As the [SLotInfo] is stored as an contigeous array within the
    /// shared memory region this accessor will allow access to indexed elements of this array.
    ///
    /// # Safety
    /// This is safe as long as the raw pointer is actually pointing to an array of the type `[SlotInfo; len]`
    pub unsafe fn new(slot_info_array: *mut SlotInfo, len: usize) -> Self {
        Self {
            inner: slot_info_array,
            len,
        }
    }
}

impl fmt::Debug for SlotInfoAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slots = unsafe { core::slice::from_raw_parts(self.inner, self.len) };
        f.debug_list().entries(slots.iter()).finish()
    }
}
