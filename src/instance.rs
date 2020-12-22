/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Instance/Client
//!
use alloc::sync::Arc;

use super::error::VchiqError;
use super::service::*;
use super::slothandler::slot_handler;
use super::state::State;
use super::VchiqResult;
use crate::doorbell;
use core::future::poll_fn;
use ruspiro_brain::spawn;
use ruspiro_console::info;
use ruspiro_error::{BoxError, Error, GenericError};

pub struct VchiqInstance {
    state: Arc<State>,
    /// Flag that indicates if both sides are in a connected state which would allow the usage of Services between ARM
    /// and VideoCore
    connected: bool,
}

impl VchiqInstance {
    /// Create a new VCHIQ instance.
    pub fn new() -> VchiqResult<Self> {
        info!("create VchiqInstance");
        let mut state = State::new()?;
        state.initialize()?;
        let state = Arc::new(state);
        // TODO: we might require a 'handle' to the future spawned here
        // as this should be cleaned up once the VCHIQ instance is dropped. Otherwise
        // this would never finish. Or we need to establish a channel between this instance
        // and the slothandler future to notify it to be stopped
        let state_clone = Arc::clone(&state);
        spawn(poll_fn(move |cx| {
            slot_handler(Arc::clone(&state_clone), cx)
        }));

        Ok(Self {
            state,
            connected: false,
        })
    }

    /// Connect the ARM side of the VCHIQ with the VideoCore side. This is a prerequisite for any further calls to
    /// VCHIQ interface that requires both sides to be connected.
    pub async fn connect(&mut self) -> VchiqResult<()> {
        info!("try to connect VchiqInstance");
        if self.connected {
            return Err(VchiqError::AlreadyConnected.into());
        }

        self.state.connect().await?;
        self.connected = true;

        Ok(())
    }

    /// Open a service between the ARM and the VideoCore side. A service is representing a specific "device" or function
    /// of the VideoCore.
    pub async fn open_service(&mut self, params: ServiceParams) -> VchiqResult<ServiceHandle> {
        if self.connected {
            self.create_service(params, true).await
        } else {
            Err(VchiqError::NotConnected.into())
        }
    }

    pub async fn create_service(
        &mut self,
        params: ServiceParams,
        open: bool,
    ) -> VchiqResult<ServiceHandle> {
        let srv_state = if open {
            ServiceState::OPENING
        } else {
            if self.connected {
                ServiceState::LISTENING
            } else {
                ServiceState::HIDDEN
            }
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
    pub async fn close_service(&mut self, service: ServiceHandle) -> VchiqResult<()> {
        self.state.close_service(service).await
    }
}

impl Drop for VchiqInstance {
    fn drop(&mut self) {
        info!("dropping VCHIQ");
    }
}
