/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Instance/Client
//!
use alloc::sync::Arc;

use super::service::*;
use super::slothandler::slot_handler;
use super::state::State;
use super::VchiqResult;
use ruspiro_brain::spawn;
use ruspiro_error::{Error, GenericError, BoxError};
use ruspiro_console::info;

pub struct VchiqInstance {
    state: Arc<State>,
    /// Flag that indicates if both sides are in a connected state which would allow the usage of Services between ARM
    /// and VideoCore
    connected: bool,
}

impl VchiqInstance {
    pub async fn new() -> VchiqResult<Self> {
        info!("create VchiqInstance");
        let mut state = State::new()?;
        state.initialize().await?;
        let state = Arc::new(state);
        spawn(slot_handler(Arc::clone(&state)));

        Ok(Self {
            state,
            connected: false,
        })
    }

    pub async fn connect(&mut self) -> VchiqResult<()> {
        info!("try to connect VchiqInstance");
        if self.connected {
            return Err(GenericError::with_message("Vchiq already connected").into());
        }

        self.state.connect().await?;
        self.connected = true;

        Ok(())
    }

    pub async fn open_service(&mut self, params: ServiceParams) -> VchiqResult<ServiceHandle> {
        if self.connected {
            self.create_service(params, true).await
        } else {
            Err(GenericError::with_message("Can't open service if not connected").into())
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
                self.state.remove_service(srv_handle);
                e
            })?;
        }

        Ok(srv_handle)
    }
}

impl Drop for VchiqInstance {
    fn drop(&mut self) {
        info!("dropping VCHIQ");
    }
}