/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SlotMessage
//!

use alloc::vec::Vec;
use core::mem;
use ruspiro_error::{BoxError, Error, GenericError};

/// The message ID of a padding message
pub const VCHIQ_MSGID_PADDING: u32 = make_msg_id(MessageType::PADDING, 0, 0);

/// A message that is stored within the [Slot]. Access to the message is only available through the [SlotAccessor]
/// functions.
#[derive(Debug)]
#[repr(C)]
pub struct SlotMessage<T: core::fmt::Debug> {
    pub header: SlotMessageHeader,
    pub data: T,
}

#[derive(Debug)]
#[repr(C)]
pub struct SlotMessageHeader {
    pub msgid: u32,
    pub size: u32,
}

#[allow(non_camel_case_types, dead_code)]
#[derive(Debug, Copy, Clone, PartialEq)]
#[repr(u32)]
pub enum MessageType {
    PADDING = 0,            /* -                                 */
    CONNECT = 1,            /* -                                 */
    OPEN = 2,               /* + (srcport, -), fourcc, client_id */
    OPENACK = 3,            /* + (srcport, dstport)              */
    CLOSE = 4,              /* + (srcport, dstport)              */
    DATA = 5,               /* + (srcport, dstport)              */
    BULK_RX = 6,            /* + (srcport, dstport), data, size  */
    BULK_TX = 7,            /* + (srcport, dstport), data, size  */
    BULK_RX_DONE = 8,       /* + (srcport, dstport), actual      */
    BULK_TX_DONE = 9,       /* + (srcport, dstport), actual      */
    PAUSE = 10,             /* -                                 */
    RESUME = 11,            /* -                                 */
    REMOTE_USE = 12,        /* -                                 */
    REMOTE_RELEASE = 13,    /* -                                 */
    REMOTE_USE_ACTIVE = 14, /* -                                 */
}

impl core::convert::From<u32> for MessageType {
    fn from(orig: u32) -> Self {
        match orig {
            0 => MessageType::PADDING,
            1 => MessageType::CONNECT,
            2 => MessageType::OPEN,
            3 => MessageType::OPENACK,
            4 => MessageType::CLOSE,
            5 => MessageType::DATA,
            6 => MessageType::BULK_RX,
            7 => MessageType::BULK_TX,
            8 => MessageType::BULK_RX_DONE,
            9 => MessageType::BULK_TX_DONE,
            10 => MessageType::PAUSE,
            11 => MessageType::RESUME,
            12 => MessageType::REMOTE_USE,
            13 => MessageType::REMOTE_RELEASE,
            14 => MessageType::REMOTE_USE_ACTIVE,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
pub struct OpenAckPayload {
    pub version: u16,
}

impl core::convert::TryFrom<SlotMessage<Vec<u8>>> for OpenAckPayload {
    type Error = BoxError;
    fn try_from(message: SlotMessage<Vec<u8>>) -> Result<Self, Self::Error> {
        if (message.header.size as usize) < core::mem::size_of::<OpenAckPayload>() {
            Err(GenericError::with_message("unable to convert into OpenAckPayload").into())
        } else {
            //let msg_ptr = &message as *const SlotMessage<Vec<u8>> as *const u8;
            //let msg = unsafe { core::ptr::read_volatile(msg_ptr as *const SlotMessage<OpenAckPayload>) };
            let data_ptr = message.data.as_ptr();
            let data = unsafe { core::ptr::read_volatile(data_ptr as *const OpenAckPayload) };

            Ok(data)
        }
    }
}

/// Create a messagid from the message type, source and target port of the message
pub const fn make_msg_id(msg_type: MessageType, src_port: u32, dst_port: u32) -> u32 {
    ((msg_type as u32) << 24) | (src_port << 12) | (dst_port << 0)
}

/// Extract the message type from the message ID
pub fn msg_type_from_id(id: u32) -> MessageType {
    let msg_type = id >> 24;
    msg_type.into()
}

pub fn dst_port_from_id(id: u32) -> u32 {
    id & 0xfff
}

pub fn src_port_from_id(id: u32) -> u32 {
    (id >> 12) & 0xfff
}

/// Calculate the padded size that a slot message with the given payload size will occupy
pub fn calc_msg_stride(payload_size: usize) -> usize {
    let hdr_size = mem::size_of::<SlotMessageHeader>();
    let msg_size = hdr_size + payload_size;

    (msg_size + hdr_size - 1) & !(hdr_size - 1)
}
