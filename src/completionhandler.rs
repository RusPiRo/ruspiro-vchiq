/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Handle service request completions
//!

use super::instance::VchiqInstance;
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use ruspiro_console::*;

pub(crate) fn completion_handler(instance: Arc<VchiqInstance>, cx: &mut Context<'_>) -> Poll<()> {
    match Box::pin(instance.await_completion(8)).as_mut().poll(cx) {
        Poll::Pending => return Poll::Pending, // waking is done in the await_completion future
        Poll::Ready(Ok(data)) => {
            info!("completion received...");
            for completion in data {
                // check for a service callback present in the completion data
                // call it

                // otherwise check for a vchi service callback present in the completion data
                // call it

                // if reason is SERVICE_CLOSED
                // call close_delivered for the service contained in the completion
            }
        },
        Poll::Ready(Err(e)) => {
            // this should never happen...
            error!("Error while awaiting completion: {:?}", e);
            return Poll::Ready(());
        },
    }

    // always return Pending! and immediately wake
    // TODO: optimize the waking!
    cx.waker().wake_by_ref();
    Poll::Pending
}
