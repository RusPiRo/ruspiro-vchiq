/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Service
//!

use super::{config::*, shared::slotmessage::SlotMessage};
use crate::types::{FourCC, Reason, ServiceHandle, ServiceParams, UserData, VchiqCallback};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::{
    any::Any,
    cell::RefCell,
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};
use ruspiro_console::*;
use ruspiro_lock::{Mutex, RWLock, Semaphore};

pub const VCHIQ_PORT_FREE: u32 = 0x1000;

/// Atomic "counter" to store the service handles
pub static NEXT_SRV_HANDLE: AtomicUsize = AtomicUsize::new(VCHIQ_MAX_SERVICES);

/// The different states a [Service] could be in.
#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ServiceOption {
    AUTOCLOSE,
    SLOT_QUOTA,
    MESSAGE_QUOTA,
    SYNCHRONOUS,
    TRACE,
}

pub struct ServiceCompletion {
    pub reason: Reason,
    /// The actual message data that has been received from the VideoCore indecating
    /// completion of the last request to this service. This message however does not only appear
    /// as part of this structure in the completion queue but also in the service message queue
    /// and thus required to be an Arc. It's not yet clear why the message is stored twice and how it is
    /// ensured the message will not beeing processed twice as a result of this.
    pub msg: Option<Arc<SlotMessage<Vec<u8>>>>,
    pub service_userdata: Option<UserData>,
    //srv_userdata: Option<Vec<u8>>, // "pointer" to the service user data passed when service was created
    //bulk_userdata: ? // TODO!
}

/// Base data of a service known to the VCIQ
pub struct ServiceBase {
    /// Service Identifier
    pub fourcc: FourCC,
    /// Service callback invoked when data was recieved from the VideoCore for this service
    pub callback: Option<Arc<VchiqCallback>>,
    /// data that shall be passed to the callback and is provided as part of the service creation
    pub userdata: Option<UserData>,
}

#[derive(Debug)]
pub struct ServiceUser {
    pub userdata: Option<UserData>,
    pub is_vchi: bool,
    pub dequeue_pending: bool,
    pub close_pending: bool,
    pub message_available_pos: isize,
    pub msg_insert: usize,
    pub msg_remove: usize,
    pub insert_event: Semaphore,
    pub remove_event: Semaphore,
    pub close_event: Semaphore,
    /// Queue of recieved messages from the videocore after a specific request.
    /// The message might also be stored in the completion queue of the VCHIQ instance
    /// and thus required to be an Arc. It's not yet clear why the message is stored twice and how it is
    /// ensured the message will not beeing processed twice as a result of this.
    /// in linux this queue is only touched in "service_callback" to add messages if the service is marked as "is_vchi"
    /// and in "dequeue_message" to remove messages if the service is marked as "is_vchi"
    /// The list is initialized with a fixed length and treated as "ring-buffer"
    pub msg_queue: Vec<Option<Arc<SlotMessage<Vec<u8>>>>>,
}

pub struct Service {
    /// Service base data
    pub base: ServiceBase,
    /// The actual service handle
    pub handle: ServiceHandle,
    /// current servic state
    pub srvstate: ServiceState,
    /// local port of the service (position in the local data slot)
    pub localport: u32,
    /// remote port of the service (position in the remote data slot)
    pub remoteport: u32,
    /// public service identifier
    pub public_fourcc: Option<FourCC>,
    /// the client id - not sure what this is for
    pub client_id: i32,
    /// Flag that the service will use automatic close after use?
    pub auto_close: bool,
    /// Flag that this is a sync service
    pub sync: bool,
    /// Flag that the service is currently closing
    pub closing: bool,
    /// Flag that the service communication shall be traced?
    pub trace: bool,
    //poll_flags: AtomicU32,
    /// current VCHIQ version of the service
    pub version: u16,
    /// minimal VCHIQ version required for the service
    pub version_min: u16,
    /// peer version?
    pub peer_version: u16,
    /// Service count - don't know what this is counting - may be obsolete
    /// if a single service instance is always wrapped with an Arc the usage/ref
    /// count is implicit and ensures the service instance is not dropped as long
    /// as it is used/refernced somewhere
    pub service_use_count: i32, // check if this is realy needed
    /// sender queue for bulk transmits
    pub bulk_tx: RWLock<BulkQueue>,
    /// receiver queue for bulk transmits
    pub bulk_rx: RWLock<BulkQueue>,
    /// event raised once the service shall be removed?
    pub remove_event: Semaphore,
    /// event raised once the bulk queue's shall be removed?
    pub bulk_remove_event: Semaphore,
    // the original implementation carries some statistic fields as well - we ommit this part for the time beeing
}

impl Drop for Service {
    fn drop(&mut self) {
        info!("drop service");
    }
}

impl Service {
    pub(crate) fn new(params: ServiceParams) -> Self {
        let service = Self {
            base: ServiceBase {
                fourcc: params.fourcc,
                callback: params.callback,
                userdata: params.userdata,
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
            bulk_tx: RWLock::new(BulkQueue::default()),
            bulk_rx: RWLock::new(BulkQueue::default()),
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
