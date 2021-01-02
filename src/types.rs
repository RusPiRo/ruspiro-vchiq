/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Types
//!
//! Types and Structures used by the VCHI and VCHIQ side
//!

use super::vchiq::shared::slotmessage::SlotMessage;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{any::Any, fmt, ops::Deref};
use ruspiro_lock::RWLock;

/// The representation of a service identifier. 4 characters represented as u32 value
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct FourCC(u32);

/// A service handle that identifies a service
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ServiceHandle(pub usize);

#[derive(Clone, Debug)]
pub struct UserData(Arc<RWLock<Box<dyn Any>>>);

impl UserData {
    pub fn new<T: core::fmt::Debug + 'static>(value: T) -> Self {
        Self(Arc::new(RWLock::new(Box::new(value))))
    }
}

impl Deref for UserData {
    type Target = Arc<RWLock<Box<dyn Any>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Reason {
    SERVICE_OPENED,        /* service, -, -             */
    SERVICE_CLOSED,        /* service, -, -             */
    MESSAGE_AVAILABLE,     /* service, header, -        */
    BULK_TRANSMIT_DONE,    /* service, -, bulk_userdata */
    BULK_RECEIVE_DONE,     /* service, -, bulk_userdata */
    BULK_TRANSMIT_ABORTED, /* service, -, bulk_userdata */
    BULK_RECEIVE_ABORTED,  /* service, -, bulk_userdata */
}

/// The type alias for a VCHIQ callback
/// TODO: last callback parameter is for bulk_userdata - need to evaluate how this will be implemented
pub type VchiqCallback =
    dyn Fn(Reason, Option<Arc<SlotMessage<Vec<u8>>>>, ServiceHandle, Option<()> /*, void*?*/);

/// Parameters to be used while adding/creating a new service
#[derive(Clone)]
pub(crate) struct ServiceParams {
    pub fourcc: FourCC,
    pub callback: Option<Arc<VchiqCallback>>,
    pub userdata: Option<UserData>,
    pub version: u16,
    pub version_min: u16,
}

impl From<&[u8; 4]> for FourCC {
    fn from(orig: &[u8; 4]) -> Self {
        Self(
            ((orig[0] as u32) << 24)
                | ((orig[1] as u32) << 16)
                | ((orig[2] as u32) << 8)
                | ((orig[3] as u32) << 0),
        )
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "'{}{}{}{}'",
            ((self.0 >> 24) & 0xFF) as u8 as char,
            ((self.0 >> 16) & 0xFF) as u8 as char,
            ((self.0 >> 8) & 0xFF) as u8 as char,
            ((self.0 >> 0) & 0xFF) as u8 as char,
        )
    }
}

impl fmt::Debug for ServiceParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceParams")
            .field("fourcc", &self.fourcc)
            .field(
                "callback",
                match &self.callback {
                    Some(_) => &"Some(callback)",
                    _ => &"None",
                },
            )
            .field("userdata", &self.userdata)
            .field("version", &self.version)
            .field("version_min", &self.version_min)
            .finish()
    }
}
