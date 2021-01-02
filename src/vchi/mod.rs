/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Video Core Host Interface API
//!
//! This is the API for the Raspberry Pi Video Core Host Interface.
//!

mod completion;
pub mod error;
mod instance;
pub mod service;

pub use error::VchiError;
pub use instance::Instance;
