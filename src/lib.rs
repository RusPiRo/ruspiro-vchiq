/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ API
//!
//! VideoCore Host Interface implementation (Q stands for Kernel?)
//!

#![no_std]
#![feature(future_poll_fn)]

extern crate alloc;
extern crate rlibc;

pub mod types;

mod vchi;
pub use vchi::*;

mod vchiq;

/*
mod config;
mod doorbell;
pub mod error;
pub mod instance;
pub mod service;
pub mod shared;
mod slothandler;
mod completionhandler;
mod state;

use ruspiro_error::BoxError;

pub type VchiqResult<T> = Result<T, BoxError>;
*/
