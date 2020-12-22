/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Service
//!

use super::config::*;
use super::shared::FourCC;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};
use ruspiro_console::*;
use ruspiro_lock::sync::{Mutex, Semaphore};

pub const VCHIQ_PORT_FREE: u32 = 0x1000;

/// Atomic "counter" to store the service handles
pub static NEXT_SRV_HANDLE: AtomicU32 = AtomicU32::new(VCHIQ_MAX_SERVICES as u32);

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ServiceHandle(pub u32);

/// The different states a [Service] could be in.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ServiceState {
    FREE,
    HIDDEN,
    LISTENING,
    OPENING,
    OPEN,
    OPENSYNC,
    CLOSESENT,
    CLOSERECVD,
    CLOSEWAIT,
    CLOSED,
}

/// The data required to create/open a VCHIQ Service
pub struct ServiceParams {
    /// Service unique name as FourCC value
    pub fourcc: FourCC,
    /// a closure that can be registered to be called on certain events of the service
    pub callback: Option<Arc<Mutex<Box<dyn FnOnce()>>>>,
    /// the requested version of the service
    pub version: u16,
    /// the minimal version the service need to be compatible with
    pub version_min: u16,
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
            .field("version", &self.version)
            .field("version_min", &self.version_min)
            .finish()
    }
}

pub struct ServiceBase {
    pub fourcc: FourCC,
}

pub struct Service {
    pub base: ServiceBase,
    pub handle: ServiceHandle,
    pub srvstate: ServiceState,
    pub localport: u32,
    pub remoteport: u32,
    pub public_fourcc: Option<FourCC>,
    pub client_id: i32,
    pub auto_close: bool,
    pub sync: bool,
    pub closing: bool,
    pub trace: bool,
    //poll_flags: AtomicU32,
    pub version: u16,
    pub version_min: u16,
    pub peer_version: u16,
    pub service_use_count: i32, // check if this is realy needed
    bulk_tx: BulkQueue,
    bulk_rx: BulkQueue,
    pub remove_event: Semaphore,
    pub bulk_remove_event: Semaphore,
}

impl Drop for Service {
    fn drop(&mut self) {
        info!("drop service");
    }
}

impl Service {
    pub fn new(params: ServiceParams) -> Self {
        let service = Self {
            base: ServiceBase {
                fourcc: params.fourcc,
            },
            handle: ServiceHandle(NEXT_SRV_HANDLE.fetch_add(1, Ordering::AcqRel)),
            srvstate: ServiceState::FREE,
            localport: VCHIQ_PORT_FREE,
            remoteport: VCHIQ_PORT_FREE,
            public_fourcc: Some(params.fourcc),
            //if let ServiceState::OPENING = srvstate { None } else { Some(params.fourcc) },
            client_id: 0,
            auto_close: true,
            sync: false,
            closing: false,
            trace: false,
            version: params.version,
            version_min: params.version_min,
            peer_version: 0,
            service_use_count: 0,
            bulk_tx: BulkQueue::default(),
            bulk_rx: BulkQueue::default(),
            remove_event: Semaphore::new(0),
            bulk_remove_event: Semaphore::new(0),
        };

        service
    }

    pub fn set_state(&mut self, new_state: ServiceState) {
        info!(
            "set new state for service {:#?} {:?} -> {:?}",
            self.base.fourcc, self.srvstate, new_state
        );
        self.srvstate = new_state;
    }
}

#[derive(Default, Debug)]
pub struct ServiceQuota {
    pub slot_quota: usize,
    pub slot_use_count: usize,
    pub message_quota: usize,
    pub message_use_count: usize,
    pub quota_event: Semaphore,
    pub previous_tx_index: isize,
}

#[derive(Default, Debug)]
pub struct BulkQueue {
    /// Where to insert the next local bulk
    local_insert: i32,
    /// Where to insert the next remote bulk (master)
    remote_insert: i32,
    /// Bulk to transfer next
    process: i32,
    /// Bulk to notify the remote client of next (master)
    remote_notify: i32,
    /// Bulk to notify the local client of, and remove, next
    remove: i32,
    bulks: [Bulk; VCHIQ_NUM_SERVICE_BULKS],
}

#[derive(Default, Copy, Clone, Debug)]
pub struct Bulk {
    mode: i16,
    dir: i16,
    //void* userdata
    handle: i32,
    //void* data
    size: usize,
    //void* remote_data
    remote_size: usize,
    actual: usize,
}
