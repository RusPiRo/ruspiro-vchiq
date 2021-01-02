/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Completions
//!
//! Handle VideoCore service request completions and invoke the service specific callbacks registered
//! from the "user side"

use super::service::Service;
use crate::{
    types::{Reason, ServiceHandle},
    vchiq::instance::Instance,
};
use alloc::{boxed::Box, collections::BTreeMap, sync::Arc};
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    cell::RefCell,
};
use ruspiro_console::*;
use ruspiro_lock::RWLock;

/// The asyncrounously call-able function (using poll_fn) that never finishes and constantly requests the VCHIQ
/// if there are service requests completed. If so, this will invoke the callbacks registered with the service
pub(crate) fn completion_handler(
    vchiq_instance: Arc<Instance>,
    services: Arc<RWLock<BTreeMap<ServiceHandle, RefCell<Service>>>>,
    cx: &mut Context<'_>,
) -> Poll<()> {
    // wait for a max number of 8 completions with each call
    match Box::pin(vchiq_instance.await_completion(8))
        .as_mut()
        .poll(cx)
    {
        // when the called future returns pending it is also responsible to register the waker
        // thus we only return pending here
        Poll::Pending => Poll::Pending,
        Poll::Ready(result) => match result {
            // if awaiting the completions produces an error we stop wating for them as some severe issue has araised on
            // VCHIQ side
            Err(e) => {
                error!("Error while awaiting completions {:?}", e);
                Poll::Ready(())
            }
            // handle the completions that have been recieved so far
            Ok(completions) => {
                //info!("received {} completions", completions.len());
                for completion in completions {
                    let user_data = completion.service_userdata.as_ref().unwrap().read();
                    let service_handle = user_data.downcast_ref::<ServiceHandle>().unwrap();
                    let service = services.read();
                    let service = service.get(&service_handle).unwrap().borrow();
                    // check for a service callback present in the completion data
                    // call it
                    if let Some(callback) = service.base.callback.as_ref() {
                        info!(
                            "invoke service callback, reason: {:?}, message {:?}",
                            completion.reason, completion.msg
                        );
                        (callback)(completion.reason, completion.msg, service.handle, None);
                    // otherwise check for a vchi service callback present in the completion data
                    // call it
                    } else if let Some(callback) = service.vchi_callback.as_ref() {
                        info!(
                            "invoke vchi service callback,  reason: {:?}, message {:?}",
                            completion.reason, completion.msg
                        );
                        (callback)(
                            service.base.userdata.as_ref().map(|ud| ud.clone()),
                            completion.reason,
                        );
                    }

                    if completion.reason == Reason::SERVICE_CLOSED { //&& instance.use_close_delivered {
                         // TODO: call close_delivered for the service contained in the completion
                    }
                }
                // once we are done with the completions wake this future and return pending
                // TODO: optimize the waking - eg. wake as soon the VCHIQ put's something new into the completion
                // list
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        },
    }
}
