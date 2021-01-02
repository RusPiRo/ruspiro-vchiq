/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ State
//!

use alloc::alloc::{alloc, dealloc, Layout};
use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;
use core::{
    cmp, mem, ptr,
    convert::{TryFrom, TryInto},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    cell::RefCell,
};
use super::config::*;
use super::service::*;
use super::shared::FourCC;
use super::shared::fragment::Fragment;
use super::shared::slot::{slot_queue_index_from_pos, Slot, SlotAccessor, SlotPosition};
use super::shared::slotmessage::*;
use ruspiro_error::{Error, GenericError};
use ruspiro_lock::r#async::{AsyncMutex, AsyncMutexGuard, AsyncSemaphore};
use ruspiro_lock::sync::Semaphore;
use ruspiro_mailbox::Mailbox;
use ruspiro_mmu::{map_memory, page_align, page_size};
use crate::{doorbell, error::VchiqError, shared::slotzero::{SlotZero, SlotZeroAccessor}};
use crate::VchiqResult;
use ruspiro_console::{error, info, warn};
use ruspiro_arch_aarch64::instructions::*;

#[allow(non_camel_case_types, dead_code)]
#[derive(Debug, Copy, Clone)]
pub enum VchiqConnectState {
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

pub struct State {
    /// flag indicating whether the [State] has been initialized
    initialized: bool,
    /// Sempahore indicating a connect message has been received
    connect: AsyncSemaphore,
    /// Semaphore used to indicate how many slots are available
    slot_available_event: Semaphore,
    /// the inner state able to be shared accross cores and to be used in async code
    inner: Arc<AsyncMutex<StateInner>>,
}

impl Drop for State {
    fn drop(&mut self) {
        info!("drop state");
    }
}

impl State {
    pub fn new() -> VchiqResult<Self> {
        let inner = StateInner::new()?;
        let slot_available_event = Semaphore::new(inner.slot_queue_available);

        Ok(Self {
            initialized: false,
            connect: AsyncSemaphore::new(0),
            slot_available_event,
            inner: Arc::new(AsyncMutex::new(inner)),
        })
    }

    pub async fn initialize(&mut self) -> VchiqResult<()> {
        if !self.initialized {
            let mut state = self.inner.lock().await;
            state.initialize()?;
            //doorbell::activate_doorbell(&self.inner);

            self.initialized = true;
        }

        Ok(())
    }

    /// get the current connection state with a short living lock on the inner state
    async fn connection_state(&self) -> VchiqConnectState {
        let state = self.inner.lock().await;
        state.conn_state
    }

    async fn set_connection_state(&self, conn_state: VchiqConnectState) {
        let mut state = self.inner.lock().await;
        state.set_conn_state(conn_state);
    }

    pub async fn connect(&self) -> VchiqResult<()> {
        if self.initialized {
            let conn_state = self.connection_state().await;
            if let VchiqConnectState::DISCONNECTED = conn_state {
                self.queue_message::<()>(None, make_msg_id(MessageType::CONNECT, 0, 0), None, true)
                    .await?;
                info!("update connection state");
                self.set_connection_state(VchiqConnectState::CONNECTING)
                    .await;
            };

            info!("next stage");
            let conn_state = self.connection_state().await;
            if let VchiqConnectState::CONNECTING = conn_state {
                info!("VCHIQ in 'connecting' state");
                self.connect.down().await;
                info!("connect semaphore down");
                self.set_connection_state(VchiqConnectState::CONNECTED)
                    .await;

                return Ok(());
            };

            info!("state was not connecting...");
            unreachable!();
        } else {
            Err(GenericError::with_message("VCHIQ State not initialized").into())
        }
    }

    pub async fn add_service(
        &self,
        srv_params: ServiceParams,
        srv_state: ServiceState,
    ) -> VchiqResult<ServiceHandle> {
        StateInner::add_service(Arc::clone(&self.inner), srv_params, srv_state).await
    }

    pub fn remove_service(&self, srv_handle: ServiceHandle) -> VchiqResult<()> {
        StateInner::remove_service(Arc::clone(&self.inner), srv_handle)
    }

    pub async fn open_service(&self, srv_handle: ServiceHandle) -> VchiqResult<()> {
        StateInner::open_service(Arc::clone(&self.inner), srv_handle).await
    }

    pub async fn close_service(&self, srv_handle: ServiceHandle) -> VchiqResult<()> {
        StateInner::close_service(Arc::clone(&self.inner), srv_handle).await
    }

    async fn queue_message<T>(
        &self,
        service: Option<Arc<RefCell<Service>>>,
        msg_id: u32,
        context: Option<T>,
        is_blocking: bool,
    ) -> VchiqResult<()>
    where T: core::fmt::Debug
    {
        StateInner::queue_message(
            Arc::clone(&self.inner),
            service,
            msg_id,
            context,
            is_blocking,
        )
        .await
    }

    pub async fn handle_slots(&self) {
        let mut state = self.inner.lock().await;
        state.slot_zero.local_mut().trigger_mut().wait().await;

        if state.poll_needed {
            // TODO: implement the service polling, as per the linux inline doc this is
            // defined as a rare condition... so keep this here empty for later implementation ;)
        }

        state.parse_rx_slots(self);
    }
}

pub(crate) struct StateInner {
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
    /// Accessor to the [SlotZero] data stored in the shared memory region
    slot_zero: SlotZeroAccessor,
    /// Accessor to the generic [Slot] data stored in the shared memory region. Keep in mind that an index 0 into the
    /// `slot_data` will point to the [SlotZero] part.
    slot_data: SlotAccessor,
    /// the actual connection state from ARM to VideoCore
    conn_state: VchiqConnectState,
    // slot index and offset to where the next message could be written to the `slot_data`
    tx_data: Option<SlotPosition>,
    // slot index and offset from where the next message can be read from the `slot_data`
    rx_data: Option<SlotPosition>,
    /// a cached copy of value from `self.slot_zero.local_ref().tx_pos()`.
    /// Only write `self.slot_zero.local_mut().set_tx_pos()`, and read `self.slot_zero.remote_ref().tx_pos()`
    local_tx_pos: u32,
    /// indicates the byte position within the slot data from where the next message will be read. The least
    /// significant bits are an index into the slot. The next bits are the index of the slot in
    /// [self.slot_zero.remote().slot_queue]
    rx_pos: u32,
    /// The slot_queue index of the slot to become available next.
    slot_queue_available: u32,

    /// The list of services known to the VCHIQ interface
    /// The elements are wrapped in a RefCell to allow individual mutable access
    /// as well as in an Arc to keep access around without keeping the inner state borrowed
    services: Vec<Option<Arc<RefCell<Service>>>>,
    /// The number of the first unused service
    unused_service: u32,

    /// The list of quota's for the services known to the VCHIQ interface
    service_quotas: Vec<ServiceQuota>,
    /// default quota for slots
    default_slot_quota: u32,
    /// default quota for messages
    default_message_quota: u32,
    /// Indicates that the slot_handler is requested to poll service specific
    /// messages. This is seen as a 'rare' condition in the linux implementation
    poll_needed: bool,
}

impl StateInner {
    fn new() -> VchiqResult<Self> {
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
            slot_zero.local_ref().slot_last() - slot_zero.local_ref().slot_first() + 1;
        let default_slot_quota = slot_queue_available / 2;
        let default_message_quota = cmp::min(default_slot_quota * 256, !0);

        let mut services = Vec::with_capacity(VCHIQ_MAX_SERVICES as usize);
        services.resize_with(VCHIQ_MAX_SERVICES as usize, || None);//Arc::new(RefCell::new(None)));

        let mut service_quotas = Vec::with_capacity(VCHIQ_MAX_SERVICES);
        service_quotas.resize_with(VCHIQ_MAX_SERVICES, || ServiceQuota::default());

        Ok(Self {
            shm_ptr,
            shm_ptr_phys,
            shm_ptr_layout: layout,
            slot_zero,
            slot_data,
            conn_state: VchiqConnectState::DISCONNECTED,
            tx_data: None,
            rx_data: None,
            local_tx_pos: 0,
            rx_pos: 0,

            slot_queue_available,
            
            services,
            unused_service: 0,

            service_quotas,
            default_slot_quota,
            default_message_quota,

            poll_needed: false,
        })
    }

    fn set_conn_state(&mut self, new_state: VchiqConnectState) {
        info!(
            "change conn_state from {:?} to {:?}",
            self.conn_state, new_state
        );
        self.conn_state = new_state;
    }

    /// Initilize the VCHIQ interface by sending the shared memory loction to the VideoCore using
    /// a mailbox call.
    ///
    /// mut access:
    ///     - slot_zero.slot data
    ///     - slot_zero.local.sync_release
    ///
    fn initialize(&mut self) -> VchiqResult<()> {
        // before notifying the VideoCore with the Slot address containing the data for the initial
        // "handshake" mark the sync_slot to be empty and available
        let slot_index = self.slot_zero.local_ref().slot_sync() as usize;
        self.slot_data.store_message(
            SlotPosition::new(slot_index, 0),
            |message: &mut SlotMessage<()>| {
                message.header.msgid = VCHIQ_MSGID_PADDING;
            },
        );
        // let the local Future handling sync_release event's pick this up
        //self.slot_zero.local_mut().sync_release_mut().signal_local();

        // ensure data is in sync and has finished writing to memory before sending the memory address to the
        // VideoCore
        dsb();
        dmb();

        // ensure data is not stuck in cache and VideoCore sees the correct values
        //flush_dcache_range(self.shm_ptr as usize, self.shm_ptr_layout.size());
        let mut mb = Mailbox::new();
        let status = mb.set_vchiq_slot_base(self.shm_ptr_phys as u32)?;

        dsb();
        dmb();

        if status != 0 {
            error!(
                "MB Error {:#x} - SlotZero:\r\n{:#?}",
                status, self.slot_zero
            );
            return Err(GenericError::with_message("unable to set VCHIQ base address").into());
        }

        Ok(())
    }

    /// Add a new service to be used for communications between ARM and VideoCore
    async fn add_service(
        this: Arc<AsyncMutex<Self>>,
        srv_params: ServiceParams,
        srv_state: ServiceState,
    ) -> VchiqResult<ServiceHandle> {
        let mut service = Service::new(srv_params);
        if let ServiceState::OPENING = srv_state {
            service.public_fourcc = None;
        }
        
        let mut srv_index = None;
        let mut state = this.lock().await;
        if let ServiceState::OPENING = srv_state {
            // find the first index with an unused service
            srv_index = state.services[..state.unused_service as usize]
                .iter()
                .enumerate()
                .find_map(
                    |(idx, service)| {
                        match service {
                            None => Some(idx as u32),
                            _ => None
                        }
                    }
                );
        } else {
            // if the requested state of the service is not "OPENING" then check the list of services in reverse
            // order that there is no service already existing using the same fourCC
            // TODO: implement the verification as in linux, skip this for the time beeing
            srv_index = state.services[..state.unused_service as usize]
                .iter()
                .rev()
                .enumerate()
                .find_map(
                    |(idx, service)| {
                        match service {
                            None => Some(idx as u32),
                            _ => None
                        }
                    }
                );
        }

        // if there could not be any free index found, check for the last unused one
        if srv_index.is_none() {
            if (state.unused_service as usize) < VCHIQ_MAX_SERVICES {
                srv_index.replace(state.unused_service);
            }
        }

        if let Some(reuse_idx) = srv_index {
            let service_handle = service.handle;
            service.localport = reuse_idx;
            info!("add new service {:#?} at index {}", service.base.fourcc, reuse_idx);
            service.set_state(srv_state);
            state.services[reuse_idx as usize].replace(Arc::new(RefCell::new(service)));
            state.unused_service += 1;

            // once we have added the new service we need to maintain it's quota
            let slot_quota = state.default_slot_quota;
            let message_quota = state.default_message_quota;
            let local_tx_pos = state.local_tx_pos as usize;
            if let Some(mut quota) = state.service_quotas.get_mut(reuse_idx as usize) {
                quota.slot_quota = slot_quota;
                quota.message_quota = message_quota;
                if quota.slot_use_count == 0 {
                    quota.previous_tx_index = slot_queue_index_from_pos(local_tx_pos) as i32 - 1;
                }
            }

            Ok(
                service_handle
            )
        } else {
            Err(
                GenericError::with_message("unable to add a new service").into()
            )
        }
    }

    pub fn remove_service(this: Arc<AsyncMutex<Self>>, handle: ServiceHandle) -> VchiqResult<()> {
        todo!("remove service not yet implemented");
    }

    pub async fn open_service(this: Arc<AsyncMutex<Self>>, handle: ServiceHandle) -> VchiqResult<()> {
        #[derive(Debug)]
        struct OpenPayload {
            fourcc: FourCC,
            client_id: i32,
            version: u16,
            version_min: u16,
        }

        let (context, localport) = {
            let state = this.lock().await;
            let service = state.service_from_handle(handle).unwrap();
            // use service
            let service = service.borrow();

            let payload = OpenPayload {
                fourcc: service.base.fourcc,
                client_id: 0,
                version: service.version,
                version_min: service.version_min,
            };
            
            (
                payload,
                service.localport
            )
        };

        StateInner::queue_message(
            Arc::clone(&this), 
            None, 
            make_msg_id(MessageType::OPEN, 
            localport, 
            0),
            Some(context),
            true).await?;

        // wait for the ACK/NACK of the message just queued
        info!("waiting for OpenService ACK/NACK");
        
        //let srvstate = StateWaitOpenServiceFuture::new(Arc::clone(&this), handle).await;
        let state = Arc::clone(&this);
        let srvstate = core::future::poll_fn(move |cx| {
            // check if we can lock the state
            match Box::pin(state.lock()).as_mut().poll(cx) {
                // locking the state successfull, now check for the service state
                Poll::Ready(locked_state) => {
                    let service = locked_state.service_from_handle(handle).unwrap();
                    if service.borrow().remove_event.try_down().is_ok() {
                        Poll::Ready(service.borrow().srvstate)
                    } else {
                        //info!("wake wait open service");
                        cx.waker().wake_by_ref(); // the Future need to be woken
                        Poll::Pending
                    }
                }
                Poll::Pending => Poll::Pending, // if the lock provides pending the lock will wake this future
            }
        }).await;
        info!("Service state: {:?}", srvstate);

        match srvstate {
            ServiceState::OPEN | ServiceState::OPENSYNC | ServiceState::CLOSEWAIT => Ok(()),
            _ => {
                error!("unable to open service, current state {:?}", srvstate);
                //let _ = State::release_service(state.clone(), handle);
                Err(
                    GenericError::with_message("unable to open service").into()
                )
            }
        }
    }

    pub async fn close_service(this: Arc<AsyncMutex<Self>>, handle: ServiceHandle) -> VchiqResult<()> {
        
        let state = this.lock().await;
        let service = state.service_from_handle(handle)?;
        info!("closing service w. local port {}", service.borrow().localport);

        match service.borrow().srvstate {
            ServiceState::FREE | ServiceState::LISTENING | ServiceState::HIDDEN => {
                // unlock_service
                return Err(VchiqError::UnableToCloseService(handle).into());
            },
            _ => (),
        }

        // mark the service closing
        service.borrow_mut().closing = true;
        // linux does now some thread syncs by aquiring the states recycle_mutex
        // guess we do not need this here...

        let srv_quota = state.service_quotas.get(service.borrow().localport as usize).unwrap();
        srv_quota.quota_event.up();
        drop(state);

        // linux does now either fire the local event to let the slot_handler picking up the real closing
        // of this service or directly invoke the call the slot_handler would do anyways.
        // state.poll_needed = true;

        // the following part is in the vchiq_close_service_internal function in linux and also called from the
        // slot_handler based on specific rare conditions... implement this here first to verify it's function
        let srvstate = service.borrow().srvstate;
        match srvstate {
            ServiceState::CLOSED |
            ServiceState::HIDDEN |
            ServiceState::LISTENING |
            ServiceState::CLOSEWAIT => {
                // called with close recieved => error
                // it's never a server! - just free the service
                StateInner::free_service(Arc::clone(&service))
            },

            ServiceState::OPENING => {
                // called with close received? -> set state to CLOSEWAIR and increase remove_event
                // otherwise
                let localport = service.borrow().localport;
                let remoteport = service.borrow().remoteport & 0x0FFF;
                StateInner::queue_message::<()>(
                    Arc::clone(&this),
                    Some(Arc::clone(&service)),
                    make_msg_id(MessageType::CLOSE, localport, remoteport),
                    None, false
                ).await?;
            },

            ServiceState::OPENSYNC | ServiceState::OPEN => {
                let localport = service.borrow().localport;
                let remoteport = service.borrow().remoteport & 0x0FFF;
                // release service_message
                if StateInner::queue_message::<()>(
                    Arc::clone(&this),
                    Some(Arc::clone(&service)),
                    make_msg_id(MessageType::CLOSE, localport, remoteport),
                    None, false
                ).await.is_ok() {
                    // if !close_recvd
                    service.borrow_mut().set_state(ServiceState::CLOSESENT);
                    // else
                    // set state to CLOSERECVD
                };

                // TODO: close_service_complete -> this would invoke a call back
                //
            },

            ServiceState::CLOSESENT => {
                todo!();
            },

            ServiceState::CLOSERECVD => {
                todo!();
            },

            _ => warn!("Close called in state {:?}", service.borrow().srvstate),

        }

        Ok(())
    }

    fn free_service(service: Arc<RefCell<Service>>) {
        let mut service = service.borrow_mut();
        match service.srvstate {
            ServiceState::OPENING |
            ServiceState::CLOSED |
            ServiceState::HIDDEN |
            ServiceState::LISTENING |
            ServiceState::CLOSEWAIT => (),

            _ => {
                error!("free service - wrong state {:?}", service.srvstate);
                return;
            },
        };

        service.set_state(ServiceState::FREE);
        service.remove_event.up();

        // TODO: this service should now beeing "dropped" and removed from the services
        // list.
    }

    /// Queue a message to be send to the VideoCore. It will be stored in the slot data of the next available slot
    ///
    /// mut access:
    ///     - slot_data
    ///     - slot_zero.local
    ///     - slot_zero.remote
    ///
    async fn queue_message<T>(
        this: Arc<AsyncMutex<Self>>,
        service: Option<Arc<RefCell<Service>>>,
        msg_id: u32,
        context: Option<T>,
        is_blocking: bool,
    ) -> VchiqResult<()>
    where T: core::fmt::Debug
    {
        let msg_type = msg_type_from_id(msg_id);
        let msg_size = context.as_ref().map_or(0, |_| mem::size_of::<T>());
        let msg_stride = calc_msg_stride(msg_size);

        let mut state = this.lock().await;

        // message handling differs on the message type
        if let MessageType::DATA = msg_type {
            // data messages always require a service
            let service = service.ok_or(VchiqError::DataMessageWithoutService)?;
            // service is not allowed to be in "closing" state

            // TODO!
            unimplemented!();
        }

        // reserve space for the new message within the slots used for transmitting data
        let slot_position = Self::reserve_space(&mut state, msg_stride, is_blocking).await?;

        // once we knew the position where to put the message we can store it there
        state
            .slot_data
            .store_message::<T, _>(slot_position, |message| {
                info!(
                    "prepare message: {:?}, pos: {:?}, size: {:?}",
                    msg_type, slot_position, msg_size
                );

                if let MessageType::DATA = msg_type {
                    unimplemented!()
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
        let tx_pos = state.local_tx_pos;
        info!("set local_tx_pos to {}", tx_pos);
        state.slot_zero.local_mut().set_tx_pos(tx_pos);
        dmb();
        dsb();

        if let Some(service) = service {
            // TODO: handling of service related messages
            if let MessageType::CLOSE = msg_type {
                // set the service state to CLOSESENT
                info!("set state CLOSESENT");
                if let Ok(mut srv) = service.try_borrow_mut() {
                    srv.set_state(ServiceState::CLOSESENT);
                } else {
                    error!("can't update service state - is already borrowed?");
                }
            }
        }

        // now it's time to inform the VideoCore to pickup the work
        state.slot_zero.remote_mut().trigger_mut().signal_remote();
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
    async fn reserve_space(
        state: &mut AsyncMutexGuard<'_, Self>,
        space_needed: usize,
        is_blocking: bool,
    ) -> VchiqResult<SlotPosition> {
        // lock the state for the duration of space reservation
        //let mut state = this.lock().await;
        // get the current position where the next message should be written that is known in the other side
        let mut tx_pos = state.local_tx_pos;
        // calculate the space remaining in the actual used Slot based on this position
        let slot_space = VCHIQ_SLOT_SIZE - (tx_pos as usize & VCHIQ_SLOT_MASK);

        info!(
            "reserve space of {} bytes from the slot  with {} bytes",
            space_needed, slot_space
        );

        if space_needed > slot_space {
            // well the requested space will not fit into the same slot that is already in use, so add
            // a padding message into the same and use the next slot
            let slot_pos = state
                .tx_data
                .as_ref()
                .expect("no slot available to be used")
                .clone();
            let size = slot_space - mem::size_of::<SlotMessageHeader>();
            state.slot_data.store_message::<(), _>(slot_pos, |message| {
                message.header.msgid = VCHIQ_MSGID_PADDING;
                message.header.size = size as u32;
            });

            // tx_pos is now pointing to the start of the next slot
            tx_pos += slot_space as u32;
        }

        // if tx_pos points to the beginning of a slot we need to request a new one to be used
        let slot_position = if (tx_pos as usize & VCHIQ_SLOT_MASK) == 0 {
            // we should never grow beyound the number of slots configured to be available for the ARM side
            assert!(tx_pos != state.slot_queue_available * VCHIQ_SLOT_SIZE as u32);

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

                if !is_blocking {
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
            let slot_queue_index =
                slot_queue_index_from_pos(tx_pos as usize) & VCHIQ_SLOT_QUEUE_MASK;
            let slot_index = state.slot_zero.local_ref().slot_queue()[slot_queue_index] as usize;

            SlotPosition::new(slot_index, 0)
        } else {
            // if we were able to reserve the message space from the slot already in use, just return its index and the
            // new offset for this. It is fine to panic here if there has not been any previous slot beeing choosen to
            // be used as this is an implementation error! As the first path should go to the upper part of this
            // if-statement
            SlotPosition::new(
                state.tx_data.unwrap().index(),
                tx_pos as usize & VCHIQ_SLOT_MASK,
            )
        };

        // as we now know the new slot position the message can be stored, update the corresponding
        // state values
        state.local_tx_pos = tx_pos + space_needed as u32;
        state.tx_data.replace(slot_position);

        info!(
            "Space reserved at {:#?} with size {}",
            slot_position, space_needed
        );

        Ok(slot_position)
    }

    fn service_from_handle(&self, srvhandle: ServiceHandle) -> VchiqResult<Arc<RefCell<Service>>> {
        let service = self.services
            .iter()
            .find(|&service| {
                match service {
                    Some(srv) if srv.borrow().handle == srvhandle => true,
                    //Some(Service { handle: srvhandle, ..}) => true,
                    _ => false,
                }
            }).map(|srv| srv.as_ref())
            .flatten()
            .map(|srv| Arc::clone(srv));

            service.ok_or(VchiqError::ServiceNotFound(srvhandle).into())
    }

    fn service_by_port(&self, local_port: u32) -> Option<Arc<RefCell<Service>>> {
        let service = self.services[local_port as usize].as_ref();
        if let Some(service) = service {
            if service.borrow().srvstate == ServiceState::FREE {
                return None;
            } else {
                return Some(Arc::clone(service));
            }
        }

        None
    }

    fn parse_rx_slots(&mut self, outer: &State) {
        let tx_pos = self.slot_zero.remote_ref().tx_pos() as u32;
        //info!("current rx_pos {:?} - remote tx_pos {:?}", state.rx_pos, tx_pos);

        while self.rx_pos != tx_pos {
            // if the states rx data does not point to anything means there is no open previous messages that needs
            // to be processed further with this package
            if self.rx_data.is_none() {
                let slot_queue_index =
                    slot_queue_index_from_pos(tx_pos as usize) as usize & VCHIQ_SLOT_QUEUE_MASK;
                info!("slot_queue_index {}", slot_queue_index);
                let slot_index =
                    self.slot_zero.remote_ref().slot_queue()[slot_queue_index] as usize;
                info!("slot_index {}", slot_index);
                self.rx_data.replace(SlotPosition::new(slot_index, 0));
                //state.rx_info = state.slot_info_from_index(slot_index);
                /*if let Some(slot_info) = state.slot_infos.get_mut(slot_index as usize) {
                    (*slot_info).use_count = 1;
                    (*slot_info).release_count = 0;
                }*/
            }

            info!("handle message from {:?}", self.rx_data);
            if let Some(rx_data) = self.rx_data {
                let slot_data = self.slot_data.read_message(rx_data);

                let msg_type = msg_type_from_id(slot_data.header.msgid);
                let local_port = dst_port_from_id(slot_data.header.msgid);
                let remote_port = src_port_from_id(slot_data.header.msgid);
                let size = slot_data.header.size;
                info!(
                    "slot header - message type {:#?} / size {:#x?}",
                    msg_type, size
                );

                // get the service the message is realted to
                let service = match msg_type {
                    MessageType::CLOSE => {
                        let service = self.service_by_port(local_port);
                        if let Some(service) = &service {
                            if service.borrow().remoteport != remote_port &&
                               service.borrow().remoteport != VCHIQ_PORT_FREE &&
                               local_port == 0 {
                                // if this is a CLOSE from a client which hadn't yet received the OPENACK
                                // we need to look for connected service
                                todo!();
                            }
                        } else {
                            // if this is a CLOSE from a client which hadn't yet received the OPENACK
                            // we need to look for connected service
                            todo!();
                        }
                        
                        service
                    },
                    MessageType::OPENACK
                    | MessageType::DATA
                    | MessageType::BULK_RX
                    | MessageType::BULK_TX
                    | MessageType::BULK_RX_DONE
                    | MessageType::BULK_TX_DONE => {
                        let service = self.service_by_port(local_port);
                        if service.is_none() {
                            // we need to skip processing here and just advance the position in the 
                            // slot...
                            todo!()
                        }

                        service
                    }
                    _ => None,
                };

                // check for header being to big for a slot
                // TODO!

                match msg_type {
                    MessageType::OPEN => unimplemented!(),
                    MessageType::OPENACK => {
                        let service = service.expect("OPENACK expects a service to be available");
                        let message: Result<OpenAckPayload, _> = slot_data.try_into();
                        if let Ok(msg) = message {
                            info!("Message: {:?}", msg);
                            service.borrow_mut().peer_version = msg.version;
                        }

                        if service.borrow().srvstate == ServiceState::OPENING {
                            service.borrow_mut().remoteport = remote_port;
                            service.borrow_mut().set_state(ServiceState::OPEN);
                            service.borrow().remove_event.up();
                        }
                    },
                    MessageType::CLOSE => {
                        if size > 0 {
                            warn!("close should not contain any data");
                        }
                        let service = service.expect("CLOSE expects a service to be available");
                        service.borrow_mut().closing = true;
                        self.service_quotas[service.borrow().localport as usize].quota_event.up();
                        //StateInner::close_service(this, handle)

                    },
                    MessageType::DATA => unimplemented!(),
                    MessageType::CONNECT => {
                        info!("CONNECT - version common {}", self.slot_zero.version());
                        // set the connect semaphore which is checked while connecting
                        // to the VCHIQ - see VchiqInstance::connect()
                        outer.connect.up();
                    }
                    MessageType::BULK_RX | MessageType::BULK_TX => unimplemented!(),
                    MessageType::BULK_RX_DONE | MessageType::BULK_TX_DONE => {}
                    MessageType::PADDING => info!("just padding..."),
                    MessageType::PAUSE => unimplemented!(),
                    MessageType::RESUME => unimplemented!(),
                    MessageType::REMOTE_USE => unimplemented!(),
                    MessageType::REMOTE_RELEASE => unimplemented!(),
                    MessageType::REMOTE_USE_ACTIVE => unimplemented!(),
                    //_ => warn!("invalid message type {:#?}", msg_type),
                }

                let msg_stride = calc_msg_stride(size as usize);
                let idx = rx_data.index();
                let offset = rx_data.offset() + msg_stride;

                self.rx_pos += msg_stride as u32;

                if (self.rx_pos as usize & VCHIQ_SLOT_MASK) == 0 {
                    // release_slot()
                    let _ = self.rx_data.take();
                } else {
                    self.rx_data.replace(SlotPosition::new(idx, offset));
                }
                info!("new rx_pos {:?}", self.rx_pos);
            }
        }
    }
}

impl Drop for StateInner {
    fn drop(&mut self) {
        info!("release shared memory {:#x}", self.shm_ptr as usize);
        unsafe { dealloc(self.shm_ptr, self.shm_ptr_layout) };
    }
}

/*
pub struct StateWaitOpenServiceFuture {
    state: Arc<AsyncMutex<StateInner>>,
    srv_handle: ServiceHandle,
}

impl StateWaitOpenServiceFuture {
    fn new(state: Arc<AsyncMutex<StateInner>>, srv_handle: ServiceHandle) -> Self {
        Self {
            state,
            srv_handle,
        }
    }
}

impl Future for StateWaitOpenServiceFuture {
    type Output = ServiceState;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // check if we can lock the state
        match Box::pin(self.state.lock()).as_mut().poll(cx) {
            // locking the state successfull, now check for the service state
            Poll::Ready(mut state) => {
                let service = state.service_from_handle(self.srv_handle).unwrap();
                if service.remove_event.try_down().is_ok() {
                    Poll::Ready(service.srvstate)
                } else {
                    //info!("wake wait open service");
                    cx.waker().wake_by_ref(); // the Future need to be woken
                    Poll::Pending
                }
            }
            Poll::Pending => Poll::Pending, // if the lock provides pending the lock will wake this future
        }
    }
}
*/