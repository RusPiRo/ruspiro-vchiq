/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHI Service
//!
//! A Service represents a specific feature/functionality/device of the videocore. Communications to this device happens
//! through messagges sent to the VideoCore using such a Service.
//!

use crate::types::{FourCC, Reason, ServiceHandle, UserData, VchiqCallback};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::any::Any;
use ruspiro_lock::RWLock;

pub type VchiCallback = dyn Fn(Option<UserData>, Reason /*reason, void* handle */);

/// Data required to be provided while creating/adding a new ARM (client) service to the VCHInterface
pub struct ServiceCreation {
    pub version: u16,
    pub version_min: u16,
    pub service_id: FourCC,
    // TODO: connection seem to contain the function pointers to the API of this service? Could be
    // expressed with a trait, maybe
    //connection: VCHI_CONNECTION_T { VCHI_CONNECTION_API*, VCHI_CONNECTION_STATE*}
    pub rx_fifo_size: usize,
    pub tx_fifo_size: usize,
    // TODO: provide propper callback signature
    pub callback: Option<Box<VchiCallback>>,
    pub callback_param: Option<UserData>,
    /// client intents to receive bulk transfers of odd length or into unaligned buffers
    pub want_unaligned_bulk_rx: bool,
    ///  client intents to transmit bulk transfers of odd length or out of unaligned buffers
    pub want_unaligned_bulk_tx: bool,
    /// client wants to check CRCs on (bulk) transfers
    pub want_crc: bool,
}

pub struct ServiceBase {
    pub fourcc: FourCC,
    pub callback: Option<Arc<VchiqCallback>>,
    pub userdata: Option<UserData>,
}

/// The attributes of a service
pub struct Service {
    pub base: ServiceBase,
    pub vchi_callback: Option<Box<VchiCallback>>,
    /// the VCHIQ handle of the service
    pub handle: ServiceHandle,
    //pub peek_size: isize,
    pub peek_buf: Option<Vec<u8>>,
    pub client_id: usize,
    pub is_client: bool,
}
