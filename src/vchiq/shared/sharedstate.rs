/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SharedState
//!
//!
use core::fmt;

use super::event::{Event, EventAccessor};
use crate::vchiq::config::*;

#[allow(non_camel_case_types, dead_code)]
#[derive(Debug)]
#[repr(u32)]
pub enum DebugInfo {
    ENTRIES = 0,
    SLOT_HANDLER_COUNT = 1,
    SLOT_HANDLER_LINE = 2,
    PARSE_LINE = 3,
    PARSE_HEADER = 4,
    PARSE_MSGID = 5,
    AWAIT_COMPLETION_LINE = 6,
    DEQUEUE_MESSAGE_LINE = 7,
    SERVICE_CALLBACK_LINE = 8,
    MSG_QUEUE_FULL_COUNT = 9,
    COMPLETION_QUEUE_FULL_COUNT = 10,
    MAX = 11,
}

/// State shared between the VideoCode and the Host (ARM CPU)
/// It's data memory layout need to 100% exactly match the expected layout within the VideoCore.
#[repr(C)]
pub struct SharedState {
    /// a non-zero value indicates the content is valid.
    initialized: u32,
    /// the first slot allocated to the owner.
    slot_first: u32,
    /// the last slot (inclusive) allocated to the owner.
    slot_last: u32,
    /// the slot llocated to synchrounous messages from the owner.
    slot_sync: u32,
    /// Signaling this event indicates that owners slot handler should execute (e.g. thread or waking the thinkable)
    trigger: Event,
    /// Indicates the byte position within the stream where the next message will be written. The least significant bits
    /// are an index into the slot. The next bits are the index of the slot in the slot_queue
    tx_pos: u32,
    /// Signaling this event if the slot is recycled.
    recycle: Event,
    /// The slot queue index where the next recycled slot will be written.
    slot_queue_recycle: u32,
    /// This event should be signalled when a synchronous message is sent.
    sync_trigger: Event,
    /// This event should be signalled when a synchronous message has been released.
    sync_release: Event,
    /// A circular buffer of slot indexes.
    slot_queue: [u32; VCHIQ_MAX_SLOTS_PER_SIDE],
    /// Debugging state
    debug: [u32; DebugInfo::MAX as usize],
}

/// As the [SharedState] data is shared between the ARM and the VideoCore special care need to be taken when reading
/// data from the memory location of this data or writing to the same. Rust should never be allowed to optimize such
/// access which will happen as the ARM side might only see writes to the data but no reads, which could lead to an
/// optimization, that removes all actual writes to the datahence to undefined behavior.
/// So this accessor struct is the only way to read and update data from an [SharedState].
pub struct SharedStateAccessor<T> {
    inner: *mut SharedState,
    trigger: EventAccessor<T>,
    recycle: EventAccessor<T>,
    sync_trigger: EventAccessor<T>,
    sync_release: EventAccessor<T>,
}

impl<T> SharedStateAccessor<T> {
    /// Create a new [SharedStateAccessor] using the raw pointer the actual [SharedState].
    ///
    /// # Safety
    /// This is safe if the raw pointer passed to this constructor is actually pointing to a memory region with data of
    /// the [SharedState] format. The lifetime of this raw pointer need to outlife the lifetime of the accessor.
    pub unsafe fn new(shared_state: *mut SharedState) -> Self {
        Self {
            inner: shared_state,
            trigger: EventAccessor::new(&mut ((*shared_state).trigger)),
            recycle: EventAccessor::new(&mut ((*shared_state).recycle)),
            sync_trigger: EventAccessor::new(&mut ((*shared_state).sync_trigger)),
            sync_release: EventAccessor::new(&mut ((*shared_state).sync_release)),
        }
    }

    volatile_getter!(initialized, u32);
    volatile_setter!(initialized, u32);

    volatile_getter!(slot_first, u32);
    volatile_setter!(slot_first, u32);

    volatile_getter!(slot_last, u32);
    volatile_setter!(slot_last, u32);

    volatile_getter!(slot_sync, u32);
    volatile_setter!(slot_sync, u32);

    volatile_getter!(tx_pos, u32);
    volatile_setter!(tx_pos, u32);

    volatile_getter!(slot_queue_recycle, u32);
    volatile_setter!(slot_queue_recycle, u32);

    volatile_getter!(slot_queue, [u32; VCHIQ_MAX_SLOTS_PER_SIDE]);
    volatile_setter!(slot_queue, [u32; VCHIQ_MAX_SLOTS_PER_SIDE]);

    volatile_getter!(debug, [u32; DebugInfo::MAX as usize]);
    volatile_setter!(debug, [u32; DebugInfo::MAX as usize]);

    pub fn trigger(&self) -> &EventAccessor<T> {
        &self.trigger
    }

    pub fn sync_trigger(&self) -> &EventAccessor<T> {
        &self.sync_trigger
    }

    pub fn sync_release(&self) -> &EventAccessor<T> {
        &self.sync_release
    }

    pub fn recycle(&self) -> &EventAccessor<T> {
        &self.recycle
    }
    pub fn trigger_mut(&mut self) -> &mut EventAccessor<T> {
        &mut self.trigger
    }

    pub fn sync_trigger_mut(&mut self) -> &mut EventAccessor<T> {
        &mut self.sync_trigger
    }

    pub fn sync_release_mut(&mut self) -> &mut EventAccessor<T> {
        &mut self.sync_release
    }

    pub fn recycle_mut(&mut self) -> &mut EventAccessor<T> {
        &mut self.recycle
    }
}

impl<T> fmt::Debug for SharedStateAccessor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedState")
            .field("initialized", &self.initialized())
            .field("slot_first", &self.slot_first())
            .field("slot_last", &self.slot_last())
            .field("slot_sync", &self.slot_sync())
            .field("trigger", &self.trigger)
            .field("tx_pos", &self.tx_pos())
            .field("recycle", &self.recycle)
            .field("slot_queue_recycle", &self.slot_queue_recycle())
            .field("sync_trigger", &self.sync_trigger)
            .field("sync_release", &self.sync_release)
            .field("slot_queue", &self.slot_queue().iter())
            .field("debug", &self.debug())
            .finish()
    }
}
