/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Slot
//!
//! A generic Slot in the VCHIQ interface to store arbitrary data. Most likely the messages passed between
//! ARM and VideoCore
//!
use super::slotmessage::{SlotMessage, SlotMessageHeader};
use crate::vchiq::config::*;
use alloc::vec::Vec;
use core::{mem, ptr, slice};
use ruspiro_console::info;

/// The generic [Slot] which memory region is shared between ARM and VideoCore.
/// It's data layout and size need to 100% match the expected VideoCore format. A [Slot] is typically a container that
/// can hold up to many messages to be exchanged between ARM and VideoCore as long as the message fit into the buffer
/// the [Slot] occupies. To ensure safe access to the contents of the [Slot] the corresponding [SlotAccessor] need to be
/// used.
#[repr(C)]
pub struct Slot {
    data: [u8; VCHIQ_SLOT_SIZE],
}

#[derive(Debug, Copy, Clone)]
pub struct SlotPosition(usize, usize);

impl SlotPosition {
    pub fn new(index: usize, offset: usize) -> Self {
        Self(index, offset)
    }

    pub fn index(&self) -> usize {
        self.0
    }

    pub fn offset(&self) -> usize {
        self.1
    }
}

/// The [SlotAccessor] is used to safely access the [Slot] data shared between the ARM and the VideoCore. As the [Slot]
///'s are stored as an array in memory and have a contigeus layout we will use the accessor to access the whole array of
/// of [Slot]s.
pub struct SlotAccessor {
    /// Raw pointer to the array of [Slot]s
    inner: *mut Slot,
    /// The len of the array the raw pointer points to
    len: usize,
}

impl Drop for SlotAccessor {
    fn drop(&mut self) {
        info!("drop slot accessor");
    }
}

impl SlotAccessor {
    /// Create the [SlotAccessor] based on the raw pointer and the desired array length.
    ///
    /// # Safety
    /// This is safe if the raw pointer is ensured to point to a memory region of the [Slot] format with at least 'len'
    /// concecutive elements.
    pub unsafe fn new(slot_array: *mut Slot, len: usize) -> Self {
        Self {
            inner: slot_array,
            len,
        }
    }

    /// Store a message in the raw slot data pointed to by the current SlotAccessor
    /// The closure passed to this function retrieves a mutable borrow to the message to be changed
    pub fn store_message<T, F>(&mut self, slot_pos: SlotPosition, f: F)
    where
        T: core::fmt::Debug + Clone,
        F: FnOnce(&mut SlotMessage<T>),
    {
        assert!(slot_pos.0 > 0);
        assert!((slot_pos.0 as usize) < self.len);
        unsafe {
            info!(
                "set message in slot {}, base ptr {:#x?}, slot start ptr {:#x?}",
                slot_pos.0,
                self.inner,
                self.inner.add(slot_pos.0)
            );
            let slot_ptr = self.inner.add(slot_pos.0);
            let msg_ptr = &mut (*slot_ptr).data[slot_pos.1 & VCHIQ_SLOT_MASK] as *mut u8
                as *mut SlotMessage<T>;
            let mut message = ptr::read_volatile(msg_ptr);

            f(&mut message);

            ptr::write_volatile(msg_ptr, message);
        }
    }

    pub fn read_message(&self, slot_pos: SlotPosition) -> SlotMessage<Vec<u8>> {
        assert!(slot_pos.0 > 0);
        assert!((slot_pos.0 as usize) < self.len);
        unsafe {
            let slot_ptr = self.inner.add(slot_pos.0);
            let msg_ptr = &mut (*slot_ptr).data[slot_pos.1 & VCHIQ_SLOT_MASK] as *mut u8
                as *mut SlotMessageHeader;
            let payload_ptr =
                (msg_ptr as *mut u8).offset(mem::size_of::<SlotMessageHeader>() as isize);
            let msg_hdr = ptr::read_volatile(msg_ptr);
            let msg_payload =
                slice::from_raw_parts_mut(payload_ptr, msg_hdr.size as usize).to_vec();

            SlotMessage {
                header: msg_hdr,
                data: msg_payload,
            }
        }
    }
}

/// calculate the position within the [Slot] array (Queue) based on the given data offset position in bytes from the
/// beginning of this array.
pub fn slot_queue_index_from_pos(pos: usize) -> usize {
    pos / VCHIQ_SLOT_SIZE
}
