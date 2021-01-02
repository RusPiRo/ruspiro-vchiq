/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ State
//!

use super::{
    config::*,
    doorbell,
    error::{VchiqError, VchiqResult},
    service::{Service, ServiceBase, ServiceQuota, ServiceState, VCHIQ_PORT_FREE},
    shared::{
        fragment::Fragment,
        slot::{slot_queue_index_from_pos, Slot, SlotAccessor, SlotPosition},
        slotmessage::*,
        slotzero::{SlotZero, SlotZeroAccessor},
    },
};
use crate::types::{FourCC, ServiceHandle, ServiceParams};
use alloc::{
    alloc::{alloc, dealloc, Layout},
    boxed::Box,
    string::ToString,
    sync::Arc,
    vec::Vec,
};
use core::{
    cell::{RefCell, RefMut},
    cmp,
    convert::{TryFrom, TryInto},
    future::{poll_fn, Future},
    mem,
    pin::Pin,
    ptr,
    task::{Context, Poll},
};
use ruspiro_arch_aarch64::instructions::*;
use ruspiro_console::{error, info, warn};
use ruspiro_error::{Error, GenericError};
use ruspiro_lock::r#async::{AsyncMutex, AsyncMutexGuard, AsyncSemaphore};
use ruspiro_lock::sync::{RWLock, Semaphore};
use ruspiro_mailbox::Mailbox;
use ruspiro_mmu::{map_memory, page_align, page_size};

#[allow(non_camel_case_types, dead_code)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ConnectState {
    DISCONNECTED,
    CONNECTING,
    CONNECTED,
    PAUSING,
    PAUSE_SENT,
    PAUSED,
    RESUMING,
    PAUSE_TIMEOUT,
    RESUME_TIMEOUT,
}

pub(crate) struct State {
    /// flag indicating whether the [State] has been initialized
    initialized: bool,
    /// Raw pointer to memory allocated to be shared between the between ARM and VideoCore. This memory region need
    /// special attributes to ensure the data updated by either side is immediately seen by the other one without any
    /// caching. This is usually ensures by setting up the MMU and configure this region to be coherent, shared and not
    /// cacheable. Another constraint: as this memory location (address) need to be shared with the VideoCore as part of
    /// a mailbox message that can only pass 32Bit addresses, this need to be in this address range!
    shm_ptr: *mut u8,
    /// Physical address to the same location that shared_memory_ptr points to, but as VideoCore BUS address which
    /// typically just means `shared_memory_ptr | 0xC000_0000` if the MMU tuns with a 1:1 mapping between virtual and
    /// physical address space.
    shm_ptr_phys: *const u8,
    /// The memory layout used to allocate the shared memory, required for deallocation
    shm_ptr_layout: Layout,
    /// The part of the state related to the Slot data that is required to be mutated individually
    pub slot_state: RWLock<SlotState>,
    /*
    /// Accessor to the [SlotZero] data stored in the shared memory region
    slot_zero: SlotZeroAccessor,
    /// Accessor to the generic [Slot] data stored in the shared memory region. Keep in mind that an index 0 into the
    /// `slot_data` will point to the [SlotZero] part.
    slot_data: SlotAccessor,
    // slot index and offset to where the next message could be written to the `slot_data`
    tx_data: Option<SlotPosition>,
    // slot index and offset from where the next message can be read from the `slot_data`
    rx_data: Option<SlotPosition>,
    /// a cached copy of value from `self.slot_zero.local_ref().tx_pos()`.
    /// Only write `self.slot_zero.local_mut().set_tx_pos()`, and read `self.slot_zero.remote_ref().tx_pos()`
    local_tx_pos: usize,
    /// indicates the byte position within the slot data from where the next message will be read. The least
    /// significant bits are an index into the slot. The next bits are the index of the slot in
    /// [self.slot_zero.remote().slot_queue]
    rx_pos: usize,
    */
    /// the actual connection state from ARM to VideoCore
    conn_state: RefCell<ConnectState>,

    /// The slot_queue index of the slot to become available next.
    slot_queue_available: usize,
    /// Indicates that the slot_handler is requested to poll service specific
    /// messages. This is seen as a 'rare' condition in the linux implementation
    poll_needed: bool,
    /// The part of the State related to the quota that requires to be individually mutated
    pub quota_state: RWLock<QuotaState>,
    /*
    // The index of the previous slot used for data messages
    previous_data_index: isize,
    /// The number of slots occupied by a data message
    data_use_count: usize,
    /// The maximum number of slots to be occupied by a data message
    data_quota: usize,
    /// The list of quota's for the services known to the VCHIQ interface
    service_quotas: Vec<ServiceQuota>,
    */
    /// The part of the State related to the services that requires to be individually mutated
    pub srv_state: RWLock<SrvState>,
    /*
    /// The list of services known to the VCHIQ interface
    /// The elements are wrapped in a RefCell to allow individual mutable access
    /// as well as in an Arc to keep access around without keeping the inner state borrowed
    services: Vec<Option<Arc<RefCell<Service>>>>,
    /// The number of the first unused service
    unused_service: usize,
    */
    /// default quota for slots
    pub default_slot_quota: usize,
    /// default quota for messages
    pub default_message_quota: usize,
    /// Sempahore indicating a connect message has been received
    pub connect: AsyncSemaphore,
    /// Semaphore used to indicate how many slots are available
    slot_available_event: Semaphore,
    slot_remove_event: Semaphore,
    data_quota_event: Semaphore,
}

unsafe impl Send for State {}
unsafe impl Sync for State {}

pub(crate) struct SlotState {
    /// Accessor to the [SlotZero] data stored in the shared memory region
    pub slot_zero: SlotZeroAccessor,
    /// Accessor to the generic [Slot] data stored in the shared memory region. Keep in mind that an index 0 into the
    /// `slot_data` will point to the [SlotZero] part.
    pub slot_data: SlotAccessor,
    // slot index and offset to where the next message could be written to the `slot_data`
    pub tx_data: Option<SlotPosition>,
    // slot index and offset from where the next message can be read from the `slot_data`
    pub rx_data: Option<SlotPosition>,
    /// a cached copy of value from `self.slot_zero.local_ref().tx_pos()`.
    /// Only write `self.slot_zero.local_mut().set_tx_pos()`, and read `self.slot_zero.remote_ref().tx_pos()`
    pub local_tx_pos: usize,
    /// indicates the byte position within the slot data from where the next message will be read. The least
    /// significant bits are an index into the slot. The next bits are the index of the slot in
    /// [self.slot_zero.remote().slot_queue]
    pub rx_pos: usize,
}

pub(crate) struct QuotaState {
    // The index of the previous slot used for data messages
    pub previous_data_index: isize,
    /// The number of slots occupied by a data message
    pub data_use_count: usize,
    /// The maximum number of slots to be occupied by a data message
    pub data_quota: usize,
    /// The list of quota's for the services known to the VCHIQ interface
    pub service_quotas: Vec<ServiceQuota>,
}

pub(crate) struct SrvState {
    /// The list of services known to the VCHIQ interface
    /// The elements are wrapped in a RefCell to allow individual mutable access
    /// as well as in an Arc to keep access around without keeping the inner state borrowed
    services: Vec<Option<Arc<RefCell<Service>>>>,
    /// The number of the first unused service
    unused_service: usize,
}

#[allow(dead_code, unused_attributes)]
impl State {
    /// Create a new State instance. This is creating a specfic memory region that will be shared
    /// between the VideoCore and the ARM as data transfer channel between the two sides. The memory
    /// region used is configured to be not cached to ensure cross core coherency (all cores and the VC
    /// does see the same content without cahce maintenance operations)
    pub(crate) fn new() -> VchiqResult<Self> {
        // The State will own a memory region that is shared between the ARM and the VideoCore. This memory region will
        // be re-used to provide different views onto it during runtime
        // the total memory we require to allocate need to be page aligned. The memory page size depends on the MMU
        // configuration and will be requested from there
        let slot_mem_size = page_align(TOTAL_SLOTS * VCHIQ_SLOT_SIZE);
        let layout = Layout::from_size_align(slot_mem_size, page_size())
            .map_err(|e| GenericError::with_message(&e.to_string()))?;
        let shm_ptr = unsafe { alloc(layout) };

        let shm_ptr_phys = ((shm_ptr as usize) | 0xC000_0000) as *const u8;
        let shm_ptr =
            unsafe { map_memory(shm_ptr, layout.size(), 0b1 << 10 | 0b10 << 8 | 0x3 << 2) };

        info!(
            "SharedMemory: {:#x} / {:#x} - size {:#}",
            shm_ptr as usize,
            shm_ptr_phys as usize,
            layout.size()
        );

        // now build the contents for the data to be shared between ARM and VideoCore
        // dependent of the alignement of the allocator and it's page size the retrieved raw pointer might not be
        // aligned as the Slots would require. Ensure the Slots and SlotZero respectively tart on an address meeting the
        // alignment requirements of the VideoCore
        let slot_zero_offset = (VCHIQ_SLOT_SIZE - shm_ptr as usize) & VCHIQ_SLOT_MASK;
        let slot_zero_ptr = unsafe { shm_ptr.offset(slot_zero_offset as isize).cast::<SlotZero>() };
        // ensure the retrieved slot zero raw pointer is aligned based on the requirements
        assert!(slot_zero_ptr as usize % VCHIQ_SLOT_SIZE == 0);

        // now create the accessor to safely initialize the SlotZero w/o any undefined behavior
        let mut slot_zero =
            unsafe { SlotZeroAccessor::new(slot_zero_ptr, slot_mem_size - slot_zero_offset) };
        // and also perform the basic initialization of the same
        let num_slots = slot_zero.initialize();
        let slot_data = unsafe {
            SlotAccessor::new(
                slot_zero_ptr.cast::<Slot>(),
                num_slots + VCHIQ_SLOT_ZERO_SLOTS,
            )
        };

        // after the basic initialization of the basic slot store the memory address of the platform specific
        // fragments used during the ARM/VC communications. So store the address of the start of the fragment data
        // as well as the number of fragments available at this address, not knowing what those fragments will be
        // used for for the time  being
        let mut platform_data = slot_zero.platform_data();
        platform_data[0] = (shm_ptr_phys as usize + slot_mem_size) as u32;
        platform_data[1] = MAX_FRAGMENTS as u32;
        slot_zero.set_platform_data(platform_data);

        // Initialization of the actual fragment data
        // the assumption is, that the list of Fragment data stores in the first 32Bit the address
        // of the next fragment entry - for whatever purpose ...
        unsafe {
            let fragments_addr =
                ((shm_ptr as usize + slot_mem_size) as *mut u8).cast::<[Fragment; MAX_FRAGMENTS]>();
            let fragments = &mut *fragments_addr;
            for c in fragments.chunks_exact_mut(MAX_FRAGMENTS - 1) {
                for f in c.iter_mut() {
                    let next_fragment = (f as *mut Fragment).offset(1);
                    let f_ptr = f as *mut Fragment as *mut u32;
                    ptr::write_volatile(f_ptr, next_fragment as u32);
                }
            }
        }

        let slot_queue_available =
            (slot_zero.local_ref().slot_last() - slot_zero.local_ref().slot_first() + 1) as usize;
        let default_slot_quota = slot_queue_available / 2;
        let default_message_quota = cmp::min(default_slot_quota * 256, !0);

        let mut services = Vec::with_capacity(VCHIQ_MAX_SERVICES as usize);
        services.resize_with(VCHIQ_MAX_SERVICES, || None); //Arc::new(RefCell::new(None)));

        let mut service_quotas = Vec::with_capacity(VCHIQ_MAX_SERVICES);
        service_quotas.resize_with(VCHIQ_MAX_SERVICES, || ServiceQuota::default());

        Ok(Self {
            initialized: false,
            shm_ptr,
            shm_ptr_phys,
            shm_ptr_layout: layout,
            slot_state: RWLock::new(SlotState {
                slot_zero,
                slot_data,
                tx_data: None,
                rx_data: None,
                local_tx_pos: 0,
                rx_pos: 0,
            }),
            connect: AsyncSemaphore::new(0),
            conn_state: RefCell::new(ConnectState::DISCONNECTED),

            slot_queue_available,

            quota_state: RWLock::new(QuotaState {
                previous_data_index: -1,
                data_use_count: 0,
                data_quota: slot_queue_available - 1,
                service_quotas,
            }),

            srv_state: RWLock::new(SrvState {
                services,
                unused_service: 0,
            }),

            default_slot_quota,
            default_message_quota,

            poll_needed: false,

            slot_available_event: Semaphore::new(slot_queue_available as u32),
            slot_remove_event: Semaphore::new(0),
            data_quota_event: Semaphore::new(0),
        })
    }

    /// Initialize the VCHIQ state and the shared memory region for usage and also notify the VideoCore
    /// about the address of this memory region to get the communication kicked off
    pub(crate) fn initialize(&mut self) -> VchiqResult<()> {
        let mut slot_state = self.slot_state.lock();
        // before notifying the VideoCore with the Slot address containing the data for the initial
        // "handshake" mark the sync_slot to be empty and available
        let slot_index = slot_state.slot_zero.local_ref().slot_sync() as usize;
        slot_state.slot_data.store_message(
            SlotPosition::new(slot_index, 0),
            |message: &mut SlotMessage<()>| {
                message.header.msgid = VCHIQ_MSGID_PADDING;
            },
        );

        // ensure data is in sync and has finished writing to memory before sending the memory address to the
        // VideoCore
        dsb();
        dmb();

        // ensure data is not stuck in cache and VideoCore sees the correct values
        // TODO: do not instatiate the Mailbox here - should be a Singleton ?
        let mut mb = Mailbox::new();
        let status = mb.set_vchiq_slot_base(self.shm_ptr_phys as u32)?;

        dsb();
        dmb();

        if status != 0 {
            error!(
                "MB Error {:#x} - SlotZero:\r\n{:#?}",
                status, slot_state.slot_zero
            );
            return Err(GenericError::with_message("unable to set VCHIQ base address").into());
        }

        self.initialized = true;
        Ok(())
    }

    /// Perform the initial handshake to the VideoCore by sending a connection request and wait for its response
    /// We need to mutate:
    /// - conn_state, slot_zero, slot_data, local_tx_pos, tx_data
    pub(crate) async fn connect(&self) -> VchiqResult<()> {
        // ensure the state and this - the shared memory region is initialized
        if !self.initialized {
            return Err(VchiqError::StateNotInitialized.into());
        }

        if *self.conn_state.borrow() == ConnectState::DISCONNECTED {
            // if we are in disconnection state send a connect message to the VideoCore
            self.queue_message::<()>(None, make_msg_id(MessageType::CONNECT, 0, 0), None, true)
                .await?;
            info!("update connection state");
            *self.conn_state.borrow_mut() = ConnectState::CONNECTING;
        }

        if *self.conn_state.borrow() == ConnectState::CONNECTING {
            // if we are in a pending connection state wait for the VC to respond to the connection request
            info!("wait for VC CONNECT");
            self.connect.down().await;
            *self.conn_state.borrow_mut() = ConnectState::CONNECTED;
        }

        Ok(())
    }

    /// Add a new Service to the state
    /// This mutates the services and quota
    pub(crate) async fn add_service(
        &self,
        srv_params: ServiceParams,
        srv_state: ServiceState,
    ) -> VchiqResult<ServiceHandle> {
        let mut service = Service::new(srv_params);
        if srv_state == ServiceState::OPENING {
            service.public_fourcc = None;
        }

        let mut srv_index = None;
        let mut service_state = self.srv_state.lock();
        // we can assume that the requested state will always be OPENING
        if srv_state == ServiceState::OPENING {
            // find the first index with an unused service
            srv_index = service_state.services[..service_state.unused_service as usize]
                .iter()
                .enumerate()
                .find_map(|(idx, service)| match service {
                    None => Some(idx),
                    _ => None,
                });
        } else {
            unreachable!();
            /*
            // if the requested state of the service is not "OPENING" then check the list of services in reverse
            // order that there is no service already existing using the same fourCC
            // TODO: implement the verification as in linux, skip this for the time beeing
            srv_index = service_state.services[..service_state.unused_service as usize]
                .iter()
                .rev()
                .enumerate()
                .find_map(|(idx, service)| match service {
                    None => Some(idx),
                    _ => None,
                });
            */
        }

        // if there could not be any free index found, check for the last unused one
        if srv_index.is_none() {
            if (service_state.unused_service as usize) < VCHIQ_MAX_SERVICES {
                srv_index.replace(service_state.unused_service);
            }
        }

        if let Some(reuse_idx) = srv_index {
            let service_handle = service.handle;
            service.localport = reuse_idx as u32;
            info!(
                "add new service {:#?} at index {}",
                service.base.fourcc, reuse_idx
            );
            service.set_state(srv_state);
            service_state.services[reuse_idx as usize].replace(Arc::new(RefCell::new(service)));
            service_state.unused_service += 1;

            // once we have added the new service we need to maintain it's quota
            let mut quota_state = self.quota_state.lock();
            let slot_quota = self.default_slot_quota;
            let message_quota = self.default_message_quota;
            let local_tx_pos = self.slot_state.read().local_tx_pos;
            if let Some(mut quota) = quota_state.service_quotas.get_mut(reuse_idx as usize) {
                quota.slot_quota = slot_quota;
                quota.message_quota = message_quota;
                if quota.slot_use_count == 0 {
                    quota.previous_tx_index = slot_queue_index_from_pos(local_tx_pos) as isize - 1;
                }
            }

            Ok(service_handle)
        } else {
            Err(VchiqError::UnableToAddService.into())
        }
    }

    pub(crate) async fn open_service(&self, srv_handle: ServiceHandle) -> VchiqResult<()> {
        #[derive(Debug, Clone)]
        struct OpenPayload {
            fourcc: FourCC,
            client_id: i32,
            version: u16,
            version_min: u16,
        }

        let (context, localport) = {
            //let state = this.lock().await;
            let service = self.service_from_handle(srv_handle).unwrap();
            // use service
            let service = service.borrow();

            let payload = OpenPayload {
                fourcc: service.base.fourcc,
                client_id: 0,
                version: service.version,
                version_min: service.version_min,
            };

            (payload, service.localport)
        };

        self.queue_message(
            None,
            make_msg_id(MessageType::OPEN, localport, 0),
            Some(context),
            true,
        )
        .await?;

        // wait for the ACK/NACK of the message just queued
        info!("waiting for OpenService ACK/NACK");

        //let srvstate = StateWaitOpenServiceFuture::new(Arc::clone(&this), handle).await;
        //let state = Arc::clone(&this);
        let srvstate = core::future::poll_fn(|cx| {
            let service = self.service_from_handle(srv_handle).unwrap();
            let service = service.borrow();
            if service.remove_event.try_down().is_ok() {
                Poll::Ready(service.srvstate)
            } else {
                //info!("wake wait open service");
                cx.waker().wake_by_ref(); // the Future need to be woken
                Poll::Pending
            }
        })
        .await;
        info!("Service state: {:?}", srvstate);

        match srvstate {
            ServiceState::OPEN | ServiceState::OPENSYNC | ServiceState::CLOSEWAIT => Ok(()),
            _ => {
                error!("unable to open service, current state {:?}", srvstate);
                //let _ = State::release_service(state.clone(), handle);
                Err(VchiqError::UnableToOpenService(srv_handle).into())
            }
        }
    }

    pub(crate) async fn close_service(&self, srv_handle: ServiceHandle) -> VchiqResult<()> {
        let service = self.service_from_handle(srv_handle)?;
        let mut service_mut = service.borrow_mut();
        info!("closing service w. local port {}", service_mut.localport);

        match service_mut.srvstate {
            ServiceState::FREE | ServiceState::LISTENING | ServiceState::HIDDEN => {
                // unlock_service
                return Err(VchiqError::UnableToCloseService(srv_handle).into());
            }
            _ => (),
        }

        // mark the service closing
        service_mut.closing = true;
        drop(service_mut);

        // close_service_internal in case we are called in the context of the slot_handler thread
        // which would never be the case, would it?
        // if we are called outside the slot_handler thread
        // call request_poll for this service with the event VCHIQ_POLL_REMOVE
        // request_poll does set some service related atomic flags and triggers the slot_handler
        // to run. For the time beeing do the "close_service_internal" right here, w/o bothering
        // the slot handler
        // wrap the content in it's own scope block it is assumed to be called with
        // close_recvd = false
        {
            let close_recvd = false;
            // the following part is in the vchiq_close_service_internal function in linux and also called from the
            // slot_handler based on specific rare conditions... implement this here first to verify it's function
            let srvstate = service.borrow().srvstate;
            match srvstate {
                ServiceState::CLOSED
                | ServiceState::HIDDEN
                | ServiceState::LISTENING
                | ServiceState::CLOSEWAIT => {
                    if close_recvd {
                        error!("close service called in state {:?}", srvstate);
                    } else
                    //if is_server - which is always false I'd guess as we are implementing the client only!
                    {
                        self.free_service(Arc::clone(&service));
                    }

                    Ok(())
                }

                ServiceState::OPENING => {
                    if close_recvd {
                        service.borrow_mut().set_state(ServiceState::CLOSEWAIT);
                        service.borrow().remove_event.up();
                        Ok(())
                    } else {
                        let localport = service.borrow().localport;
                        let remoteport = service.borrow().remoteport & 0x0FFF;
                        self.queue_message::<()>(
                            Some(Arc::clone(&service)),
                            make_msg_id(MessageType::CLOSE, localport, remoteport),
                            None,
                            false,
                        )
                        .await
                    }
                }

                ServiceState::OPENSYNC | ServiceState::OPEN => {
                    if close_recvd {
                        // TODO: abort bulks
                    }

                    // release_service_message(service)

                    let localport = service.borrow().localport;
                    let remoteport = service.borrow().remoteport & 0x0FFF;
                    // release service_message
                    if self
                        .queue_message::<()>(
                            Some(Arc::clone(&service)),
                            make_msg_id(MessageType::CLOSE, localport, remoteport),
                            None,
                            false,
                        )
                        .await
                        .is_ok()
                    {
                        if !close_recvd {
                            service.borrow_mut().set_state(ServiceState::CLOSESENT);
                        }
                        // else
                        // set state to CLOSERECVD
                    };

                    Ok(())
                    // TODO: close_service_complete -> this would invoke a call back
                    //
                }

                ServiceState::CLOSESENT => {
                    todo!();
                }

                ServiceState::CLOSERECVD => {
                    todo!();
                }

                _ => {
                    warn!("Close called in state {:?}", service.borrow().srvstate);
                    Ok(())
                }
            }
        }?;

        // now wait for the remove_event triggered for the service
        poll_fn(|cx| {
            let service = self.service_from_handle(srv_handle).unwrap();
            let service = service.borrow();
            if service.remove_event.try_down().is_ok() {
                if service.srvstate == ServiceState::FREE
                    || service.srvstate == ServiceState::OPEN
                    || service.srvstate == ServiceState::LISTENING
                {
                } else {
                    warn!(
                        "close service {} remains in state {:?}",
                        service.localport, service.srvstate
                    );
                }

                Poll::Ready(
                    if service.srvstate != ServiceState::FREE
                        && service.srvstate != ServiceState::LISTENING
                    {
                        Err(VchiqError::UnableToCloseService(srv_handle).into())
                    } else {
                        Ok(())
                    },
                )
            } else {
                //info!("wake wait open service");
                cx.waker().wake_by_ref(); // the Future need to be woken
                Poll::Pending
            }
        })
        .await
    }

    /// Finalizing the service closure
    pub(crate) fn close_service_complete(
        &self,
        service: Arc<RefCell<Service>>,
        failstate: ServiceState,
    ) -> VchiqResult<()> {
        let srvstate = service.borrow().srvstate;
        match srvstate {
            ServiceState::OPEN | ServiceState::CLOSESENT | ServiceState::CLOSERECVD => {
                service.borrow_mut().set_state(ServiceState::CLOSED);
            }
            ServiceState::LISTENING => (),
            _ => {
                error!(
                    "close service complete called in service state {:?}",
                    srvstate
                );
                return Err(VchiqError::UnableToCloseService(service.borrow().handle).into());
            }
        };

        // TODO: make service callback with message SERVICE_CLOSED
        // if this call fails set the failstate of the service and return

        // TODO: check if this is really required, this seem to clean up all "manually" counted usages
        // of the service? Should be enough to remove the service from the service list
        // and thus the Arc shall release the memory of the service as soon as all Arc references goes out of
        // scope - we need to verify that this will be actually the case and no one is holding
        // a service reference any longer ...
        let use_count = service.borrow().service_use_count;
        for _ in 0..use_count {
            // release_service_internal(service);
        }

        service.borrow_mut().client_id = 0;
        service.borrow_mut().remoteport = VCHIQ_PORT_FREE;
        let srvstate = service.borrow().srvstate;
        if srvstate == ServiceState::CLOSED {
            self.free_service(Arc::clone(&service));
        } else if srvstate != ServiceState::CLOSEWAIT {
            service.borrow().remove_event.up();
        };

        Ok(())
    }

    fn free_service(&self, service: Arc<RefCell<Service>>) {
        info!("free service");
        let mut service = service.borrow_mut();
        match service.srvstate {
            ServiceState::OPENING
            | ServiceState::CLOSED
            | ServiceState::HIDDEN
            | ServiceState::LISTENING
            | ServiceState::CLOSEWAIT => (),

            _ => {
                error!("free service - wrong state {:?}", service.srvstate);
                return;
            }
        };

        service.set_state(ServiceState::FREE);
        service.remove_event.up();

        // TODO: this service should now beeing "dropped" and removed from the services
        // list.
    }

    /// Queue a message to the VideoCore.
    /// This will mutate:
    /// slot_zero, slot_data, local_tx_pos and tx_data
    /// previous_data_index, data_use_count, service_quotas of one service
    pub(crate) async fn queue_message<T: core::fmt::Debug + Clone>(
        &self,
        service: Option<Arc<RefCell<Service>>>,
        msg_id: u32,
        context: Option<T>,
        block: bool,
    ) -> VchiqResult<()> {
        let msg_type = msg_type_from_id(msg_id);
        let msg_size = context.as_ref().map_or(0, |_| mem::size_of::<T>());
        let msg_stride = calc_msg_stride(msg_size);

        info!("queue message {:?}", msg_type);
        // slot_mutex lock
        if msg_type == MessageType::DATA {
            let slot_state = self.slot_state.read();
            let quota_state = self.quota_state.read();
            // This message type can only be queued in a context of a service
            // slot_mutex unlock on error
            let service = service
                .as_ref()
                .ok_or(VchiqError::DataMessageWithoutService)?;
            let service = service.borrow_mut();
            // this service is not allowed to be closed
            if service.closing {
                // slot_mutex unlock
                return Err(VchiqError::ServiceAlreadyClosed(service.handle).into());
            }

            let service_quota = quota_state
                .service_quotas
                .get(service.localport as usize)
                .unwrap();
            // lock quota spinlock
            let mut tx_end_index =
                slot_queue_index_from_pos(slot_state.local_tx_pos + msg_stride - 1) as isize;
            // ensure data messages don't use more than their quota of slots
            while (tx_end_index as isize) != quota_state.previous_data_index
                && quota_state.data_use_count == quota_state.data_quota
            {
                // unlock quota spinlock
                // slot_mutex unlock
                self.data_quota_event.down(); // TODO: wait as async?
                                              // lock quota spinlock
                                              // slot_mutex lock
                tx_end_index =
                    slot_queue_index_from_pos(slot_state.local_tx_pos + msg_stride - 1) as isize;
                if (tx_end_index as isize) == quota_state.previous_data_index
                    || quota_state.data_use_count < quota_state.data_quota
                {
                    self.data_quota_event.up();
                    break;
                }
            }

            while service_quota.message_use_count == service_quota.message_quota
                || (tx_end_index != service_quota.previous_tx_index
                    && service_quota.slot_use_count == service_quota.slot_quota)
            {
                // unlock quota spinlock
                // slot_mutex unlock
                service_quota.quota_event.down(); // TODO: wait async?
                if service.closing {
                    return Err(VchiqError::ServiceClosing.into());
                }
                // slot_mutex lock
                if service.srvstate != ServiceState::OPEN {
                    // slot_mutex unlock
                    return Err(VchiqError::ServiceAlreadyClosed(service.handle).into());
                }
                // lock quota spinlock
                tx_end_index =
                    slot_queue_index_from_pos(slot_state.local_tx_pos + msg_stride - 1) as isize;
            }
            //unlock quota spinlock
        }

        // reserve space for the new message within the slots used for transmitting data
        let slot_position = self.reserve_space(msg_stride, block).await?;

        let mut slot_state = self.slot_state.lock();
        info!("slot locked");
        let mut quota_state = self.quota_state.lock();
        info!("quota locked");
        let tx_end_index = slot_queue_index_from_pos(slot_state.local_tx_pos - 1) as isize;
        // once we knew the position where to put the message we can store it there
        slot_state
            .slot_data
            .store_message::<T, _>(slot_position, |message| {
                info!(
                    "prepare message: {:?}, pos: {:?}, size: {:?}",
                    msg_type, slot_position, msg_size
                );

                if let MessageType::DATA = msg_type {
                    // if message context data is given copy this to the payload
                    // can this fail?
                    if let Some(cx) = context {
                        info!("copy message context");
                        message.data = cx;
                    }
                    // lock quota_spinlock

                    // if this transmission can't fit in the last slot used by any service, the data_use_count
                    // must be increased
                    if tx_end_index != quota_state.previous_data_index {
                        quota_state.previous_data_index = tx_end_index;
                        quota_state.data_use_count += 1;
                    }

                    // unwrap is fine here as we would never get here if this service does not exist (checked at the
                    // beginning of this method)
                    let service = (&service).as_ref().unwrap().borrow();
                    let service_quota = quota_state
                        .service_quotas
                        .get_mut(service.localport as usize)
                        .unwrap();

                    // if the current slot is not the same last used by this service, the service's slot_use_count
                    // must be increased
                    if tx_end_index != service_quota.previous_tx_index {
                        service_quota.previous_tx_index = tx_end_index;
                        service_quota.slot_use_count += 1;
                    }
                // unlock quota_spinlock
                } else {
                    // if message context data is given copy this to the payload
                    if let Some(cx) = context {
                        info!("copy message context");
                        message.data = cx;
                    }
                }

                message.header.msgid = msg_id;
                message.header.size = msg_size as u32;
            });
        // write memory barrier to ensure write operation has finished and data is visible to the other side
        dmb();
        dsb();

        // now let the other side know about the new message put into the slot data by updating the local tx_pos
        let tx_pos = slot_state.local_tx_pos;
        info!("set local_tx_pos to {}", tx_pos);
        slot_state.slot_zero.local_mut().set_tx_pos(tx_pos as u32);
        dmb();
        dsb();

        if let Some(ref service) = service {
            // TODO: handling of service related messages
            if msg_type == MessageType::CLOSE {
                // set the service state to CLOSESENT
                info!("set state CLOSESENT");
                service.borrow_mut().set_state(ServiceState::CLOSESENT);
            }
        }

        // now it's time to inform the VideoCore to pickup the work
        slot_state
            .slot_zero
            .remote_mut()
            .trigger_mut()
            .signal_remote();
        info!("message queued and VC notified");

        Ok(())
    }

    /// Reserve the required space within the actual or next free slot
    ///
    /// mut access
    ///     - slot_data
    ///     - slot_zero.local
    ///     - slot_zero.remote
    ///     - local_tx_pos
    ///     - tx_data
    async fn reserve_space(&self, space_needed: usize, block: bool) -> VchiqResult<SlotPosition> {
        let mut slot_state = self.slot_state.lock();
        // get the current position where the next message should be written that is known in the other side
        let mut tx_pos = slot_state.local_tx_pos;
        // calculate the space remaining in the actual used Slot based on this position
        let slot_space = VCHIQ_SLOT_SIZE - (tx_pos & VCHIQ_SLOT_MASK);

        info!(
            "reserve space of {} bytes from the slot  with {} bytes",
            space_needed, slot_space
        );

        if space_needed > slot_space {
            // well the requested space will not fit into the same slot that is already in use, so add
            // a padding message into the same and use the next slot
            let slot_pos = slot_state
                .tx_data
                .as_ref()
                .expect("no slot available to be used")
                .clone();
            let size = slot_space - mem::size_of::<SlotMessageHeader>();
            slot_state
                .slot_data
                .store_message::<(), _>(slot_pos, |message| {
                    message.header.msgid = VCHIQ_MSGID_PADDING;
                    message.header.size = size as u32;
                });

            // tx_pos is now pointing to the start of the next slot
            tx_pos += slot_space;
        }

        // if tx_pos points to the beginning of a slot we need to request a new one to be used
        let slot_position = if (tx_pos & VCHIQ_SLOT_MASK) == 0 {
            // we should never grow beyound the number of slots configured to be available for the ARM side
            assert!(tx_pos != self.slot_queue_available * VCHIQ_SLOT_SIZE);

            /*
            // Slot availability is signaled with a semaphore, so check if there is one available
            if state.slot_available_event.try_down().is_err() {
                // no -> there is actually no Slot available
                // update statistics
                //self.stats.slot_stalls += 1;
                // before actually waiting for a new slot to become available or just return with the unavailability
                // info, flush the current slot's (may contain only the PADDING message from above) to the VideoCore
                // once the slots are processed they are likely to be recycled and therefore free to be used again
                state.local_tx_pos = tx_pos;
                state.slot_zero.local_mut().set_tx_pos(tx_pos);
                state.slot_zero.remote_mut().trigger_mut().signal_remote();
                info!(
                    "VC informed to progress slot processing until tx_pos {}",
                    tx_pos
                );

                if !block {
                    // if it is not requested to wait for the next free slot return to the caller that there was
                    // nothing available - this will also release the lock of the state
                    return Err(GenericError::with_message("no slots available").into());
                }

                // TODO:
                // here we will now actually await an available slot. This should not keep the lock on state
                // so we release the lock and request it in an async fashion and only if we could get the lock we will
                // try to check the availability semaphore
                if state.slot_available_event.try_down().is_err() {
                    // there is no slot available yet so retry the space reservation.
                    // we keep the current state as "Entry" as will need to recalculate the tx_pos and
                    // slot positions as they might change while waiting for a free slot
                    return Err(GenericError::with_message(
                        "waiting for slots not yet implemented",
                    )
                    .into());
                }
            }
            */

            // getting here we know we have a Slot we are allowed to use so calculate the index and
            // offset into this slot where the message will be stored, the masking of the index ensures
            // a cirular queue usage
            let slot_queue_index = slot_queue_index_from_pos(tx_pos) & VCHIQ_SLOT_QUEUE_MASK;
            let slot_index =
                slot_state.slot_zero.local_ref().slot_queue()[slot_queue_index] as usize;

            SlotPosition::new(slot_index, 0)
        } else {
            // if we were able to reserve the message space from the slot already in use, just return its index and the
            // new offset for this. It is fine to panic here if there has not been any previous slot beeing choosen to
            // be used as this is an implementation error! As the first path should go to the upper part of this
            // if-statement
            SlotPosition::new(
                slot_state.tx_data.unwrap().index(),
                tx_pos & VCHIQ_SLOT_MASK,
            )
        };

        // as we now know the new slot position the message can be stored, update the corresponding
        // state values
        slot_state.local_tx_pos = tx_pos + space_needed;
        slot_state.tx_data.replace(slot_position);

        info!(
            "Space reserved at {:#?} with size {}",
            slot_position, space_needed
        );

        Ok(slot_position)
    }

    pub(crate) fn service_by_port(&self, local_port: u32) -> Option<Arc<RefCell<Service>>> {
        let srv_state = self.srv_state.read();
        let service = srv_state.services[local_port as usize].as_ref();
        if let Some(service) = service {
            if service.borrow().srvstate == ServiceState::FREE {
                return None;
            } else {
                return Some(Arc::clone(service));
            }
        }

        None
    }

    pub(crate) fn service_from_handle(
        &self,
        srvhandle: ServiceHandle,
    ) -> VchiqResult<Arc<RefCell<Service>>> {
        let srv_state = self.srv_state.read();
        let service = srv_state
            .services
            .iter()
            .find(|&service| {
                match service {
                    Some(srv) if srv.borrow().handle == srvhandle => true,
                    //Some(Service { handle: srvhandle, ..}) => true,
                    _ => false,
                }
            })
            .map(|srv| srv.as_ref())
            .flatten()
            .map(|srv| Arc::clone(srv));

        service.ok_or(VchiqError::ServiceNotFound(srvhandle).into())
    }
}

impl Drop for State {
    fn drop(&mut self) {
        info!("drop state");
        let ptr = (self.shm_ptr_phys as usize) & !0xC000_0000;
        info!("release shared memory {:#x}", ptr);
        unsafe { dealloc(ptr as *mut u8, self.shm_ptr_layout) };
    }
}
