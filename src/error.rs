/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Errors
//!

use crate::service::ServiceHandle;
use core::fmt;
use ruspiro_error::*;

#[derive(Debug)]
pub enum VchiqError {
    StateNotInitialized,
    AlreadyConnected,
    NotConnected,
    ServiceNotFound(ServiceHandle),
    ServiceClosing,
    ServiceAlreadyClosed(ServiceHandle),
    UnableToAddService,
    UnableToOpenService(ServiceHandle),
    UnableToCloseService(ServiceHandle),
    DataMessageWithoutService,
}

impl Error for VchiqError {}

impl fmt::Display for VchiqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VCHIQ-Error: {:?}", self)
    }
}
