/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Completion
//!

use crate::{
    types::Reason,
    vchiq::{
        config::MAX_COMPLETIONS,
        service::{ServiceCompletion, ServiceUser},
        shared::slotmessage::SlotMessage,
    },
};
use alloc::{sync::Arc, vec::Vec};
use ruspiro_console::*;
use ruspiro_lock::Semaphore;

pub struct Completion {
    pub completion_insert: usize,
    pub completion_remove: usize,
    pub insert_event: Semaphore,
    pub remove_event: Semaphore,
    /// The list of completions is updated in linux at two places:
    /// The "service_callback" adds items with the corresponding service and message to this queue
    /// while the "await_completion" removes them.
    /// The list is initialized with a fixed length and treated as "ring-buffer"
    pub completions: Vec<Option<ServiceCompletion>>,
}

impl Default for Completion {
    fn default() -> Self {
        let mut completions = Vec::with_capacity(MAX_COMPLETIONS);
        completions.resize_with(MAX_COMPLETIONS, Default::default);

        Completion {
            completion_insert: 0,
            completion_remove: 0,
            insert_event: Semaphore::new(0),
            remove_event: Semaphore::new(0),
            completions,
        }
    }
}

impl Completion {
    pub fn add_completion(
        &mut self,
        reason: Reason,
        message: Option<Arc<SlotMessage<Vec<u8>>>>,
        user_service: &mut ServiceUser,
    ) {
        // add completion
        let mut insert = self.completion_insert;
        let remove = self.completion_remove;
        // in case we are out of space wait for the "client" to process the completion queue
        // TODO: do this in an async wait fashion
        if insert - remove >= MAX_COMPLETIONS {
            // out of space
            warn!("completion queue full");
            if self.remove_event.try_down().is_err() {
                // Poll::Pending
            }
            // Poll::Ready
        }

        let completion = self
            .completions
            .get_mut(insert & (MAX_COMPLETIONS - 1))
            .unwrap();
        completion.replace(ServiceCompletion {
            reason,
            msg: message,
            service_userdata: user_service.userdata.as_ref().map(|ud| ud.clone()),
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
        self.completion_insert = insert;

        info!("completion added, insert event raised");
        self.insert_event.up();
    }
}
