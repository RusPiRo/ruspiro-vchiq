/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHI Errors
//!

use core::fmt;
use ruspiro_error::{BoxError, Error};

use crate::types::ServiceHandle;

pub type VchiResult<T> = Result<T, BoxError>;

#[derive(Debug)]
pub enum VchiError {
    NotInitialized,
    ServiceNotFound(ServiceHandle),
}

impl Error for VchiError {}

impl fmt::Display for VchiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
