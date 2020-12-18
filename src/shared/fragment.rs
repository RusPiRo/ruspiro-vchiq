/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Fragment
//!

/// It's not yet clear what this fragment are used for. It's assumed they are not shared with the VC but do have the
/// same memory attribute requirements and are therefore stored close to the VchiqSlotZero in memory
const CACHE_LINE_SIZE: usize = 32;

#[derive(Debug)]
#[repr(C)]
pub struct Fragment {
    headbuf: [u8; CACHE_LINE_SIZE],
    tailbuf: [u8; CACHE_LINE_SIZE],
}
