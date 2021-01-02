/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SlotZero
//!
use core::fmt;
use core::mem;

use super::event::{LocalEvent, RemoteEvent};
use super::sharedstate::{DebugInfo, SharedState, SharedStateAccessor};
use super::slotinfo::{SlotInfo, SlotInfoAccessor};
use super::FourCC;
use crate::vchiq::config::*;
use ruspiro_console::*;
use ruspiro_lock::sync::Semaphore;

/// The [SlotZero] has to have the 100% same layout and memory footprint as expected by the VideoCore. Access to parts
/// of the data can only be given with the respective accessor to ensure the data is properly updated in the shared
/// memory region this data structure is located at and now compiler optimizations or the like can lead to undefined
/// behavior, as sometime the ARM side will only see writes to parts of this structure - where the VC is doing the reads
/// or thr ARM side will only see reads of parts of this structure - where the VC does the writes.
#[repr(C)]
pub struct SlotZero {
    /// The magic of the zero slot should always be the FOURCC representation of 'VCHI'.
    magic: FourCC,
    /// The vchiq version this implementation supports
    version: u16,
    /// The minimal vchiq version that could be supported by the actual implementation
    version_min: u16,
    /// The actual byte size of this slot zero representation
    slot_zero_size: u32,
    /// The byte size of a slot (should always be 4096)
    slot_size: u32,
    /// The number of "data" slots that should be available for the VCHIQ communication
    max_slots: u32,
    /// The number of "data" slots each side (ARM/VC) can occupy (usually half of max_slots)
    max_slots_per_side: u32,
    /// storage place for platform specific data. This is used on the ARM side to put an offset to the platform
    /// fragments and the number of fragments located at this offset position. It's not clear whether the VC side
    /// also uses this content.
    platform_data: [u32; 2],
    /// The state of the VCHIQ master - this is in our scenario alsways the VideCore side - 32bytes till here
    master: SharedState,
    /// The state of the VCHIQ slave - this is in our scenario always the ARM side
    slave: SharedState,
    /// Current info (counters) for each slot that might be used by either side
    slots: [SlotInfo; VCHIQ_MAX_SLOTS],
}

/// The SlotZeroAccessor is the only way to safely read and update the contents of the SlotZero stored within the
/// memory reagion shared between ARM and VideoCore
pub struct SlotZeroAccessor {
    inner: *mut SlotZero,
    local: SharedStateAccessor<LocalEvent>,
    remote: SharedStateAccessor<RemoteEvent>,
    slots: SlotInfoAccessor,
    total_memory: usize,
}

impl Drop for SlotZeroAccessor {
    fn drop(&mut self) {
        info!("drop slot zero accessor");
    }
}

impl SlotZeroAccessor {
    /// Create a new [SlotZeroAccessor] given the raw pointer to a [SlotZero] and the total memory sized allocated to
    /// cover all slots. It returns the total number of slots that are available for message transafers based on the
    /// memory size available.
    ///
    /// # Safety
    /// This is safe if the raw pointer points to a memory region of the format of a [SlotZero] with at least the size
    /// of the same and a lifetime that outlives the accessor.
    pub unsafe fn new(slot_zero: *mut SlotZero, total_memory: usize) -> Self {
        let local = SharedStateAccessor::new(&mut (*slot_zero).slave);
        let remote = SharedStateAccessor::new(&mut (*slot_zero).master);
        let slots = SlotInfoAccessor::new(&mut (*slot_zero).slots[0], VCHIQ_MAX_SLOTS);
        Self {
            inner: slot_zero,
            local,
            remote,
            slots,
            total_memory,
        }
    }

    /// Initialize the SlotZero sealed by this accessor. It returns the number of slots available for the interface
    ///
    pub fn initialize(&mut self) -> usize {
        let slot_zero_size = mem::size_of::<SlotZero>();
        // calculate the index of the first slot after the SlotZero based on the size of the SlotZero
        let first_data_slot = (slot_zero_size + VCHIQ_SLOT_SIZE - 1) / VCHIQ_SLOT_SIZE;
        let num_slots = (self.total_memory / VCHIQ_SLOT_SIZE) - first_data_slot;
        // verify that memory for at least 4 slots in addition to the SlotZero has been allocated
        assert!(num_slots >= 4, "VCHIQ interface requires at least 4 slots");
        self.set_magic(b"VCHI".into());
        self.set_version(8);
        self.set_version_min(3);
        self.set_slot_zero_size(slot_zero_size as u32);
        self.set_slot_size(VCHIQ_SLOT_SIZE as u32);
        self.set_max_slots(VCHIQ_MAX_SLOTS as u32);
        self.set_max_slots_per_side(VCHIQ_MAX_SLOTS_PER_SIDE as u32);

        // provide the index for each side (master/slave) where the different slot types start within the
        // allocated memory for all slots
        self.remote_mut().set_slot_sync(first_data_slot as u32);
        self.remote_mut()
            .set_slot_first((first_data_slot + 1) as u32);
        self.remote_mut()
            .set_slot_last((first_data_slot + (num_slots / 2) - 1) as u32);

        self.local_mut()
            .set_slot_sync((first_data_slot + (num_slots / 2)) as u32);
        self.local_mut()
            .set_slot_first((first_data_slot + (num_slots / 2) + 1) as u32);
        self.local_mut()
            .set_slot_last((first_data_slot + num_slots - 1) as u32);
        self.local_mut().set_tx_pos(0);

        // once the slot indices are set prepare the initial local slot_queue
        let mut slot_queue = self.local_ref().slot_queue();
        let first = self.local_ref().slot_first();
        let last = self.local_ref().slot_last();
        let slot_queue_available = (last - first) as u32 + 1;
        for i in 0..slot_queue_available {
            slot_queue[i as usize] = i + first;
        }
        self.local_mut().set_slot_queue(slot_queue);

        // configure the local debug settings
        let mut debug = self.local_ref().debug();
        debug[DebugInfo::ENTRIES as usize] = DebugInfo::MAX as u32;
        self.local_mut().set_debug(debug);

        // finally initialize the events of the ARM side
        self.local_mut().trigger_mut().init(Some(Semaphore::new(0)));
        self.local_mut()
            .sync_trigger_mut()
            .init(Some(Semaphore::new(0)));
        self.local_mut()
            .sync_release_mut()
            .init(Some(Semaphore::new(0)));
        self.local_mut().recycle_mut().init(Some(Semaphore::new(0)));

        self.local_mut().set_initialized(1);

        num_slots
    }

    volatile_getter!(magic, FourCC);
    volatile_setter!(magic, FourCC);

    volatile_getter!(version, u16);
    volatile_setter!(version, u16);

    volatile_getter!(version_min, u16);
    volatile_setter!(version_min, u16);

    volatile_getter!(slot_zero_size, u32);
    volatile_setter!(slot_zero_size, u32);

    volatile_getter!(slot_size, u32);
    volatile_setter!(slot_size, u32);

    volatile_getter!(max_slots, u32);
    volatile_setter!(max_slots, u32);

    volatile_getter!(max_slots_per_side, u32);
    volatile_setter!(max_slots_per_side, u32);

    volatile_getter!(platform_data, [u32; 2]);
    volatile_setter!(platform_data, [u32; 2]);

    pub fn local_ref(&self) -> &SharedStateAccessor<LocalEvent> {
        &self.local
    }

    pub fn local_mut(&mut self) -> &mut SharedStateAccessor<LocalEvent> {
        &mut self.local
    }

    pub fn remote_ref(&self) -> &SharedStateAccessor<RemoteEvent> {
        &self.remote
    }

    pub fn remote_mut(&mut self) -> &mut SharedStateAccessor<RemoteEvent> {
        &mut self.remote
    }

    pub fn slots_ref(&self) -> &SlotInfoAccessor {
        &self.slots
    }

    pub fn slots_mut(&mut self) -> &mut SlotInfoAccessor {
        &mut self.slots
    }
}

impl fmt::Debug for SlotZeroAccessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlotZero")
            .field("magic", &self.magic())
            .field("version", &self.version())
            .field("version_min", &self.version_min())
            .field("slot_zero_size", &self.slot_zero_size())
            .field("slot_size", &self.slot_size())
            .field("max_slots", &self.max_slots())
            .field("max_slots_per_side", &self.max_slots_per_side())
            .field("platform_data", &self.platform_data())
            .field("master/remote", &self.remote)
            .field("slave/local", &self.local)
            .field("slots", &self.slots)
            .finish()
    }
}
