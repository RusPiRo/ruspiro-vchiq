/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Instance
//!

mod completion;

use super::{
    config::{MAX_COMPLETIONS, MSG_QUEUE_SIZE},
    error::{VchiqError, VchiqResult},
    service::{Service, ServiceCompletion, ServiceState, ServiceUser},
    shared::slotmessage::{make_msg_id, MessageType, SlotMessage},
    slothandler::slot_handler,
    state::State,
};
use crate::types::{Reason, ServiceHandle, ServiceParams, UserData};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use completion::Completion;
use core::{
    cell::RefCell,
    convert::{TryFrom, TryInto},
    fmt::Debug,
    future::poll_fn,
    task::Poll,
};
use ruspiro_brain::spawn;
use ruspiro_console::*;
use ruspiro_error::GenericError;
use ruspiro_lock::{RWLock, Semaphore};

pub(crate) struct Instance {
    /// The actual internal state of the interface.
    pub state: Arc<State>,
    /// Flag that indicates if both sides are in a connected state which would allow the usage of Services between ARM
    /// and VideoCore
    connected: Arc<RWLock<bool>>,

    /// completion data
    completion: Arc<RWLock<Completion>>,
}

impl Instance {
    pub(crate) fn new() -> VchiqResult<Arc<Self>> {
        info!("create the VCHIQ Instance");
        let mut state = State::new()?;
        state.initialize()?;
        let state = Arc::new(state);
        // TODO: we might require a 'handle' to the future spawned here
        // as this should be cleaned up once the VCHIQ instance is dropped. Otherwise
        // this would never finish. Or we need to establish a channel between this instance
        // and the slothandler future to notify it to be stopped
        // in the current design the VCHI API is a singleton that instantiates this once. The singleton lives
        // as long as the RPi is powered so the clean-up usecase can be deferred for the time beeing
        let state_clone = Arc::clone(&state);
        spawn(poll_fn(move |cx| {
            slot_handler(Arc::clone(&state_clone), cx)
        }));

        Ok(Arc::new(Self {
            state,
            connected: Arc::new(RWLock::new(false)),
            completion: Arc::new(RWLock::new(Completion::default())),
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

        Ok(())
    }

    pub async fn create_service(
        self: Arc<Self>,
        params: ServiceParams,
        open: bool, // this might be always true as we are always the "client" side and thus open the service
        // only the VC side beeing the "server" would set the service to "LISTEN"
        vchi: bool,
    ) -> VchiqResult<ServiceHandle> {
        let srv_state = if open {
            // the instance is required to be connected before we can create a service
            if !*self.connected.read() {
                return Err(VchiqError::NotConnected.into());
            }
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
        let completion = Arc::clone(&self.completion);
        // create a user_service instance
        // store the original userdata inside this instance and pass the user_service
        // as user_data to the service addition
        let mut msg_queue = Vec::with_capacity(MSG_QUEUE_SIZE);
        msg_queue.resize_with(MSG_QUEUE_SIZE, Default::default);
        let user_service = ServiceUser {
            userdata: params.userdata,
            is_vchi: vchi,
            dequeue_pending: false,
            close_pending: false,
            message_available_pos: -1, //instance.completion_remove - 1
            msg_insert: 0,
            msg_remove: 0,
            insert_event: Semaphore::new(0),
            remove_event: Semaphore::new(0),
            close_event: Semaphore::new(0),
            msg_queue,
        };
        let this = Arc::clone(&self);
        let params = ServiceParams {
            callback: Some(Arc::new(
                move |reason, message, srv_handle, bulk_userdata| {
                    let service = this.state.service_from_handle(srv_handle).unwrap();
                    service_callback(
                        reason,
                        message,
                        bulk_userdata,
                        service,
                        Arc::clone(&completion),
                    );
                },
            )),
            userdata: Some(UserData::new(user_service)),
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

    pub async fn close_service(self: Arc<Self>, service_handle: ServiceHandle) -> VchiqResult<()> {
        self.state.close_service(service_handle).await
    }

    /// Queue a message to the video core
    pub async fn queue_message<T: core::fmt::Debug + Clone>(
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

    /// Dequeue a message means retreiving the results from the VideoCore once the corresponding request to the
    /// service has been completed. A "client" will typically register a callback to get notified once the VC answered
    /// to a previous request. Once this callback is invoked the "client" will either signal a semaphore to request the
    /// result in another thread or directly call the dequeue_message to retrieve the results.
    /// The dequeue_message can only be called on VCHI services
    pub async fn dequeue_message<T>(
        self: &Arc<Self>,
        service: ServiceHandle,
        max_data_to_read: usize,
    ) -> VchiqResult<T>
    where
        T: Debug + TryFrom<SlotMessage<Vec<u8>>>,
        <T as TryFrom<SlotMessage<Vec<u8>>>>::Error: Debug,
    {
        let service = self.state.service_from_handle(service)?;
        let user_data = service.borrow().base.userdata.as_ref().unwrap().clone();
        {
            let user_data = user_data.read();
            let user_service = user_data.downcast_ref::<ServiceUser>().unwrap();

            if !user_service.is_vchi {
                return Err(GenericError::with_message(
                    "called dequeue for a service that is not marked as vchi",
                )
                .into());
            }
        }

        poll_fn(|cx| {
            let mut user_data = user_data.lock();
            let user_service = user_data.downcast_mut::<ServiceUser>().unwrap();
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

        let mut user_data = user_data.lock();
        let user_service = user_data.downcast_mut::<ServiceUser>().unwrap();

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
        let result = (*message).clone().try_into().unwrap(); //.map_err(|e| GenericError::with_message("unable to convert message").into())?;
        user_service.remove_event.up();

        // TODO: release_message(service_handle, message);

        Ok(result)
    }

    /// Wait until we've received the completion of a service request to the VideoCore.
    /// The completion list is filled from the "service_callback" that is invoked for all
    /// vchi services when a DATA message is received and handled in the "slothandler".
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
            // TODO: do we really need to do the following?
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

fn service_callback(
    reason: Reason,
    message: Option<Arc<SlotMessage<Vec<u8>>>>,
    bulk_userdata: Option<()>,
    service: Arc<RefCell<Service>>,
    completion: Arc<RWLock<Completion>>,
) -> VchiqResult<()> {
    info!("internal service callback called with reason {:?}", reason);
    let mut skip_completion = false;
    let service = service.borrow();
    let mut user_data = service.base.userdata.as_ref().unwrap().lock();
    let mut user_service = user_data.downcast_mut::<ServiceUser>().unwrap();

    let message = Arc::new(message);
    if user_service.is_vchi {
        // TODO: there might be no message when the callback is called? In this case the message should be an
        // Option!
        //if let Some(message) = message {
        // TODO: loop as long as this is true but in an async fashion, requires
        // the whole callback to be async!
        if user_service.msg_insert == (user_service.msg_remove + MSG_QUEUE_SIZE) {
            info!("service_callback - message queue is full");
            // if there is no MESSAGE_AVAILABLE entry in the completion queue,
            // add one:
            if user_service.message_available_pos < completion.read().completion_remove as isize {
                info!("inserting an extra MESSAGE_AVAILABLE into completio queue");
                completion.lock().add_completion(reason, None, user_service);
            }

            if user_service.remove_event.try_down().is_err() {
                // Poll::Pending
            }
            // Poll::Ready
        }
        let msg_insert = user_service.msg_insert;
        *user_service
            .msg_queue
            .get_mut(msg_insert & (MSG_QUEUE_SIZE - 1))
            .unwrap() = (*message).as_ref().map(|m| Arc::clone(&m));
        user_service.msg_insert += 1;

        // if there is another "thread" waiting in DEQUEUE_MESSAGE, or if there is a MESSAGE_AVAILABLE in the
        // completion queue then bypass a further addition to the completion queue
        if user_service.message_available_pos >= completion.read().completion_remove as isize
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
        completion.lock().add_completion(
            reason,
            (*message).as_ref().map(|m| Arc::clone(m)),
            user_service,
        );
    }

    Ok(())
}

/*
fn add_completion(
    completions: Arc<RWLock<Completion>>,
    reason: Reason,
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

    if reason == Reason::MESSAGE_AVAILABLE {
        user_service.message_available_pos = insert as isize;
    }

    insert += 1;
    completions.completion_insert = insert;

    completions.insert_event.up();
}
*/
