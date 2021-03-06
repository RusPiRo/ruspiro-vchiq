/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Instance/Client
//!
use super::config::*;
use super::error::VchiqError;
use super::service::*;
use super::shared::slotmessage::*;
use super::slothandler::slot_handler;
use super::completionhandler::completion_handler;
use super::state::State;
use super::VchiqResult;
use crate::{doorbell, service};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::{poll_fn, Future};
use core::task::Poll;
use ruspiro_brain::spawn;
use ruspiro_console::*;
use ruspiro_error::{BoxError, Error, GenericError};
use ruspiro_lock::RWLock;
use ruspiro_lock::Semaphore;

pub struct VchiqInstance {
    state: Arc<State>,
    /// Flag that indicates if both sides are in a connected state which would allow the usage of Services between ARM
    /// and VideoCore
    connected: Arc<RWLock<bool>>,

    /// The completion data secured with a ReadWrite Lock
    completion: Arc<RWLock<Completion>>,
}

struct Completion {
    completion_insert: usize,
    completion_remove: usize,
    insert_event: Semaphore,
    remove_event: Semaphore,
    /// The list of completions is updated in linux at two places:
    /// The "service_callback" adds items with the corresponding service and message to this queue
    /// while the "await_completion" removes them.
    /// The list is initialized with a fixed length and treated as "ring-buffer"
    completions: Vec<Option<ServiceCompletion>>,
}

impl VchiqInstance {
    /// Create a new VCHIQ instance.
    pub fn new() -> VchiqResult<Arc<Self>> {
        info!("create VchiqInstance");
        let mut state = State::new()?;
        state.initialize()?;
        let state = Arc::new(state);
        // TODO: we might require a 'handle' to the future spawned here
        // as this should be cleaned up once the VCHIQ instance is dropped. Otherwise
        // this would never finish. Or we need to establish a channel between this instance
        // and the slothandler future to notify it to be stopped
        let state_clone = Arc::clone(&state);
        spawn(
            poll_fn(move |cx| slot_handler(Arc::clone(&state_clone), cx))
        );

        let mut completions = Vec::with_capacity(MAX_COMPLETIONS);
        completions.resize_with(MAX_COMPLETIONS, Default::default);

        Ok(Arc::new(Self {
            state,
            connected: Arc::new(RWLock::new(false)),
            completion: Arc::new(RWLock::new(Completion {
                completion_insert: 0,
                completion_remove: 0,
                insert_event: Semaphore::new(0),
                remove_event: Semaphore::new(0),
                completions,
            })),
        }))
    }

    /// Connect the ARM side of the VCHIQ with the VideoCore side. This is a prerequisite for any further calls to
    /// VCHIQ interface that requires both sides to be connected.
    pub async fn connect(self: &Arc<Self>) -> VchiqResult<()> {
        info!("try to connect VchiqInstance");
        if *self.connected.read() {
            return Err(VchiqError::AlreadyConnected.into());
        }

        self.state.connect().await?;
        *self.connected.lock() = true;

        // once we are connected we spawn a thought that will check for completed service messages sent to the VCHIQ
        // and invoke it's corresponding callbacks. In linux this runs in the user_mode side to invoke the user_mode
        // callbacks
        let self_clone = Arc::clone(self);
        spawn(poll_fn::<(), _>(move |cx| completion_handler(Arc::clone(&self_clone), cx))
        );

        Ok(())
    }

    /// Open a service between the ARM and the VideoCore side. A service is representing a specific "device" or function
    /// of the VideoCore.
    pub async fn open_service(
        self: &Arc<Self>,
        params: ServiceParams,
    ) -> VchiqResult<ServiceHandle> {
        if *self.connected.read() {
            self.create_service(params, true).await
        } else {
            Err(VchiqError::NotConnected.into())
        }
    }

    pub async fn create_service(
        self: &Arc<Self>,
        params: ServiceParams,
        open: bool, // this might be always true as we are always the "client" side and thus open the service
                    // only the VC side beeing the "server" would set the service to "LISTEN"
    ) -> VchiqResult<ServiceHandle> {
        let srv_state = if open {
            ServiceState::OPENING
        } else {
            if *self.connected.read() {
                ServiceState::LISTENING
            } else {
                ServiceState::HIDDEN
            }
        };

        // when creating a service from "outside" we will define a fixed internal service callback
        // if the client requires a callbeck it need to be registered as part of it's userdata parameter
        // this will also require special handling from the client side to call "await_completion" to be able to call
        // it's callback - but this await is not called service specific - maybe a dequeue_message would be a better fit
        // This is not yet clear how this works finally together - currently focussing on how the vchi-tests are
        // implemented and trying to get them to run successfully!
        let completions = Arc::clone(&self.completion);
        let params = ServiceParams {
            callback: Some(Box::new(move |service, reason, message| {
                service_callback(service, Arc::clone(&completions), reason, message);
            })),
            ..params
        };
        let srv_handle = self.state.add_service(params, srv_state).await?;
        if open {
            self.state.open_service(srv_handle).await.map_err(|e| {
                //self.state.remove_service(srv_handle);
                e
            })?;
        }

        Ok(srv_handle)
    }

    /// Close a previously opened service
    pub async fn close_service(self: &Arc<Self>, service: ServiceHandle) -> VchiqResult<()> {
        self.state.close_service(service).await
    }

    /// Queue a message to the video core
    pub async fn queue_message<T: core::fmt::Debug>(
        self: &Arc<Self>,
        service: ServiceHandle,
        data: T,
    ) -> VchiqResult<()> {
        let service = self.state.service_from_handle(service)?;
        let local_port = service.borrow().localport;
        let remote_port = service.borrow().remoteport;
        self.state
            .queue_message(
                Some(service),
                make_msg_id(MessageType::DATA, local_port, remote_port),
                Some(data),
                false,
            )
            .await
    }

    pub async fn dequeue_message(
        self: &Arc<Self>,
        service: ServiceHandle,
        max_data_to_read: usize,
    ) -> VchiqResult<Vec<u8>> {
        let service = self.state.service_from_handle(service)?;
        /*
        service.peek_size  >= 0 {
            // this will copy the service.peek_buf of service.peek_size to return
            // the actual data - need to investigate how this data is provided/filled
            // as this is available in the user space vchi service attributes only
        }
        */
        if !service.borrow().base.userdata.is_vchi {
            return Err(
                GenericError::with_message("called dequeue for is_vchi = false service").into(),
            );
        }

        poll_fn(|cx| {
            let mut service = service.borrow_mut();
            let user_service = &mut service.base.userdata;
            if user_service.msg_remove == user_service.msg_insert {
                user_service.dequeue_pending = true;
                // now wait until data has been inserted into the service msg_queue
                if user_service.insert_event.try_down().is_ok() {
                    if user_service.msg_remove != user_service.msg_insert {
                        return Poll::Ready(());
                    }
                }

                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;

        let mut service = service.borrow_mut();
        let user_service = &mut service.base.userdata;
        if user_service.msg_remove > user_service.msg_insert {
            return Err(GenericError::with_message("user service msg_remove > msg_insert").into());
        }

        let msg_remove = user_service.msg_remove;
        user_service.msg_remove += 1;
        let message = user_service
            .msg_queue
            .get_mut(msg_remove & (MSG_QUEUE_SIZE - 1))
            .unwrap()
            .take()
            .unwrap();
        let result = message.data.clone(); //.iter().map(|v| *v).collect::<Vec<u8>>();
        user_service.remove_event.up();

        // TODO: release_message(service_handle, message);

        Ok(result)
    }

    pub fn set_service_option(
        self: &Arc<Self>,
        service: ServiceHandle,
        option: ServiceOption,
        value: i32,
    ) -> VchiqResult<()> {
        let service = self.state.service_from_handle(service)?;
        let mut service = service.borrow_mut();
        match option {
            ServiceOption::AUTOCLOSE => service.auto_close = value != 0,
            ServiceOption::SLOT_QUOTA => {
                let mut quota_state = self.state.quota_state.lock();
                let srv_quota = quota_state
                    .service_quotas
                    .get_mut(service.localport as usize)
                    .unwrap();
                let value = if value == 0 {
                    self.state.default_slot_quota
                } else {
                    value as usize
                };
                if value >= srv_quota.slot_use_count && value < usize::MAX {
                    srv_quota.slot_quota = value;
                    if srv_quota.message_quota >= srv_quota.message_use_count {
                        // signal the service it may have dropped below its quota
                        srv_quota.quota_event.up();
                    }
                }
            }
            ServiceOption::MESSAGE_QUOTA => {
                let mut quota_state = self.state.quota_state.lock();
                let srv_quota = quota_state
                    .service_quotas
                    .get_mut(service.localport as usize)
                    .unwrap();
                let value = if value == 0 {
                    self.state.default_message_quota
                } else {
                    value as usize
                };
                if value >= srv_quota.message_use_count && value < usize::MAX {
                    srv_quota.message_quota = value;
                    if srv_quota.slot_quota >= srv_quota.slot_use_count {
                        // signal the service it may have dropped below its quota
                        srv_quota.quota_event.up();
                    }
                }
            }
            ServiceOption::SYNCHRONOUS => {
                if service.srvstate == ServiceState::HIDDEN
                    || service.srvstate == ServiceState::LISTENING
                {
                    service.sync = value != 0;
                }
            }
            ServiceOption::TRACE => service.trace = value != 0,
        }

        Ok(())
    }

    pub async fn await_completion(
        self: &Arc<Self>,
        count: usize,
    ) -> VchiqResult<Vec<ServiceCompletion>> {
        if !(*self.connected.read()) {
            return Err(VchiqError::NotConnected.into());
        }

        // wait for the event signaling something has been actually completed
        poll_fn(|cx| {
            let completions = self.completion.read();
            if completions.completion_remove == completions.completion_insert {
                let _ = completions.insert_event.try_down(); // TODO: use async sema to optimize waking
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                info!("completion event received");
                Poll::Ready(())
            }
        })
        .await;

        let mut result = Vec::new();
        let mut completions = self.completion.lock();
        let mut remove = completions.completion_remove;
        // try to return as many completion records as requested
        for _ in 0..count {
            if remove == completions.completion_insert {
                // no more completion records known - leave now
                break;
            }

            let completion = completions
                .completions
                .get_mut(remove & (MAX_COMPLETIONS - 1))
                .unwrap()
                .take()
                .unwrap();
            // now we have the actual completion record
            // grab the user service data from the service packet into the completion userdata
            // completion.srv_userdata = ((completion.srv_userdata as Service).base.userdata as ServiceUser).userdata;

            // next grab the SlotMessage that was put into the completion (if any) and provide it as a result
            // we migth not need to do anything as this is part of completion
            result.push(completion);
            remove += 1;
            completions.completion_remove = remove;
        }

        Ok(result)
    }
}

impl Drop for VchiqInstance {
    fn drop(&mut self) {
        info!("dropping VCHIQ");
    }
}

fn service_callback(
    service: Arc<RefCell<Service>>,
    completions: Arc<RWLock<Completion>>,
    reason: CallbackReason,
    message: SlotMessage<Vec<u8>>,
) -> VchiqResult<()> {
    info!("internal service callback called with reason {:?}", reason);
    let mut skip_completion = false;
    let user_service = &mut service.borrow_mut().base.userdata;
    let message = Arc::new(message);
    if user_service.is_vchi {
        // TODO: there might be no message when the callback is called? In this case the message should be an Option!
        //if let Some(message) = message {
        // TODO: loop as long as this is true but in an async fashion, requires
        // the whole callback to be async!
        if user_service.msg_insert == (user_service.msg_remove + MSG_QUEUE_SIZE) {
            info!("service_callback - message queue is full");
            // if there is no MESSAGE_AVAILABLE entry in the completion queue,
            // add one:
            if user_service.message_available_pos < completions.read().completion_remove as isize {
                info!("inserting an extra MESSAGE_AVAILABLE into completio queue");
                add_completion(Arc::clone(&completions), reason, None, user_service);
            }

            if user_service.remove_event.try_down().is_err() {
                // Poll::Pending
            }
            // Poll::Ready
        }
        let msg_insert = user_service.msg_insert;
        user_service
            .msg_queue
            .get_mut(msg_insert & (MSG_QUEUE_SIZE - 1))
            .unwrap()
            .replace(Arc::clone(&message));
        user_service.msg_insert += 1;

        // if there is another "thread" waiting in DEQUEUE_MESSAGE, or if there is a MESSAGE_AVAILABLE in the
        // completion queue then bypass a further addition to the completion queue
        if user_service.message_available_pos >= completions.read().completion_remove as isize
            || user_service.dequeue_pending
        {
            user_service.dequeue_pending = false;
            //skip completion
            skip_completion = true;
        }

        user_service.insert_event.up();
        //}
    };

    if !skip_completion {
        info!("add completion");
        // add this data to the completion list if not already done
        // but this time add the message to the completion
        add_completion(
            Arc::clone(&completions),
            reason,
            Some(Arc::clone(&message)),
            user_service,
        );
    }

    Ok(())
}

fn add_completion(
    completions: Arc<RWLock<Completion>>,
    reason: CallbackReason,
    message: Option<Arc<SlotMessage<Vec<u8>>>>,
    user_service: &mut ServiceUser,
) {
    // add completion
    let mut insert = completions.read().completion_insert;
    let remove = completions.read().completion_remove;
    // in case we are out of space wait for the "client" to process the completion queue
    // TODO: do this in an async wait fashion
    if insert - remove >= MAX_COMPLETIONS {
        // out of space
        warn!("completion queue full");
        if completions.read().remove_event.try_down().is_err() {
            // Poll::Pending
        }
        // Poll::Ready
    }

    let mut completions = completions.lock();
    let completion = completions
        .completions
        .get_mut(insert & (MAX_COMPLETIONS - 1))
        .unwrap();
    completion.replace(ServiceCompletion {
        reason,
        msg: message,
        //srv_userdata: user_service.userdata,
    });

    if reason == Reason::SERVICE_CLOSED {
        // TODO: "lock" service - keeping an instance until this CLOSE is delivered
        // should not be necessare if the service is wrapped in an Arc - it's kept around until the last
        // reference is dropped...
        /*
        if completions.use_close_delivered {
            user_service.close_pending = true;
        }
        */
    }

    // do we need a memory barrier here?
    // dmb();

    if reason == CallbackReason::MESSAGE_AVAILABLE {
        user_service.message_available_pos = insert as isize;
    }

    insert += 1;
    completions.completion_insert = insert;

    completions.insert_event.up();
}
