/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Shared Data structures
//!
use core::fmt;

#[macro_use]
mod macros;
mod event;
pub mod fragment;
mod sharedstate;
pub mod slot;
mod slotinfo;
pub mod slotmessage;
pub mod slotzero;

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(transparent)]
pub struct FourCC(u32);

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
