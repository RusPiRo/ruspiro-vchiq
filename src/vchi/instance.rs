/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHI Instance
//!
//! Such an instance will only exist once and provides the access to the underlying functions of the
//! VideoCore Host Interface

use super::{
    completion::completion_handler,
    error::VchiResult,
    service::{Service, ServiceBase, ServiceCreation, VchiCallback},
};
use crate::{
    types::{ServiceHandle, ServiceParams, UserData},
    vchiq::{instance as vchiq, shared::slotmessage::SlotMessage},
    VchiError,
};
use alloc::{boxed::Box, collections::BTreeMap, sync::Arc, vec::Vec};
use ruspiro_error::GenericError;
use core::{any::Any, cell::RefCell, convert::TryFrom, fmt::Debug, future::poll_fn, sync::atomic::{AtomicUsize, Ordering}};
use ruspiro_brain::spawn;
use ruspiro_console::*;
use ruspiro_lock::RWLock;
//use ruspiro_singleton::Singleton;

/*
/// The Singleton of the VCHI Instance
pub static INSTANCE: Singleton<Instance> = Singleton::lazy(&|| {
    Instance::new()
});
*/

const MAX_INSTANCE_SERVICES: usize = 32;
static SERVICE_HANDLE: AtomicUsize = AtomicUsize::new(0);

pub struct Instance {
    initialized: bool,
    connected: bool,
    pub use_close_delivered: bool,
    // completion_thread => will be a Thought representing the handling of completions of vchiq service requests
    used_services: usize, // TODO: is this required with the growable list of services?
    /// The list of services known to this instance, typically only MAX_INSTANCE_SERVICES shall be added
    pub services: Arc<RWLock<BTreeMap<ServiceHandle, RefCell<Service>>>>,
    /// The shared reference to the internal VCHIQ instance that actually implements the communication with the
    /// VideoCore
    pub(crate) vchiq_instance: Arc<vchiq::Instance>,
}

impl Instance {
    pub fn new() -> Self {
        // failing to initialize the VCHIQ instance is not recoverable, so panic is fine if this failes
        // there is no alternative for the VCHI interface to be properly setup w/o and ther seem to be a severe
        // implementation issue
        let vchiq_instance = vchiq::Instance::new().expect("failed to initialize VCHIQ Instance!");
        Instance {
            initialized: false,
            connected: false,
            use_close_delivered: false,
            used_services: 0,
            services: Arc::new(RWLock::new(BTreeMap::new())),
            vchiq_instance,
        }
    }

    pub fn initialize(&mut self) {
        if !self.initialized {
            // this was introduced with VCHIQ version 8 - we do not implement any checks here as
            // the VCHI and the VCHIQ version are always the same and implemented side-by-side. This is not the case
            // in linux as both parts are split into user space (VCHI) and kernel space (VCHIQ) - thus user space
            // required to request the kernel space version to check which features could ba activated
            self.use_close_delivered = true;
            self.initialized = true;
        }
    }

    pub async fn connect(&mut self) -> VchiResult<()> {
        if !self.connected {
            self.vchiq_instance.connect().await?;
            self.connected = true;
            // once we are connected we spawn a thought that will check for completed service messages sent to the VCHIQ
            // and invoke it's corresponding callbacks. In linux this runs in the user_mode side to invoke the user_mode
            // callbacks
            let services = Arc::clone(&self.services);
            let instance = Arc::clone(&self.vchiq_instance);
            spawn(poll_fn::<(), _>(move |cx| {
                completion_handler(Arc::clone(&instance), Arc::clone(&services), cx)
            }));
        }

        Ok(())
    }

    pub async fn service_open(&mut self, setup: ServiceCreation) -> VchiResult<ServiceHandle> {
        let params = ServiceParams {
            fourcc: setup.service_id,
            userdata: setup.callback_param,
            version: setup.version,
            version_min: setup.version_min,
            callback: None,
        };
        self.create_service(params, setup.callback, true).await
    }

    pub async fn service_close(&mut self, handle: ServiceHandle) -> VchiResult<()> {
        info!("closing service");
        {
            let services = self.services.read();
            let service = services
                .get(&handle)
                .ok_or(VchiError::ServiceNotFound(handle))?;
            let handle = service.borrow().handle;
            Arc::clone(&self.vchiq_instance)
                .close_service(handle)
                .await?;
            info!("vchiq service closed, now remove vchi side");
        }

        self.services.lock().remove(&handle);

        Ok(())
    }

    pub async fn remove_service(&mut self, handle: ServiceHandle) -> VchiResult<()> {
        todo!()
    }

    async fn create_service(
        &mut self,
        params: ServiceParams,
        callback: Option<Box<VchiCallback>>,
        open: bool,
    ) -> VchiResult<ServiceHandle> {
        if !self.initialized {
            return Err(VchiError::NotInitialized.into());
        }

        // create a new handle for the new service atmically, thus it's kept unique!
        let handle = ServiceHandle(SERVICE_HANDLE.fetch_add(1, Ordering::AcqRel));
        // now create the service on VCHIQ side. Instead of passing a pointer to the user side of the service
        // definition as linux does, we just pass the handle as userdata. The VCHIQ side will not investigate the
        // userdata anyway
        let userdata = params.userdata.clone();
        let vchiq_params = ServiceParams {
            userdata: Some(UserData::new(handle)),
            ..params
        };
        let vchiq_handle = Arc::clone(&self.vchiq_instance)
            .create_service(vchiq_params.clone(), open, vchiq_params.callback.is_none())
            .await?;

        let service = Service {
            base: ServiceBase {
                fourcc: vchiq_params.fourcc,
                callback: vchiq_params.callback,
                userdata,
            },
            vchi_callback: callback,
            peek_buf: None,
            is_client: open,
            client_id: 0,
            handle: vchiq_handle,
        };

        self.services.lock().insert(handle, RefCell::new(service));

        Ok(handle)
    }

    pub async fn queue_message<T: core::fmt::Debug + Clone>(
        &self,
        handle: ServiceHandle,
        data: T,
    ) -> VchiResult<()> {
        info!("queue message for service {:?}, data {:?}", handle, data);
        let services = self.services.read();
        let service = services
            .get(&handle)
            .ok_or(VchiError::ServiceNotFound(handle))?;
        let handle = service.borrow().handle;
        Arc::clone(&self.vchiq_instance)
            .queue_message(handle, data)
            .await
    }

    pub async fn dequeue_message(
        &self,
        handle: ServiceHandle,
        max_data_to_read: usize,
    ) -> VchiResult<Vec<u8>>
    /*where
        T: Debug + TryFrom<SlotMessage<Vec<u8>>>,
        <T as TryFrom<SlotMessage<Vec<u8>>>>::Error: Debug,*/
    {
        info!("dequeue message for service {:?}", handle);
        let mut services = self.services.lock();
        let mut service = services
            .get(&handle)
            .ok_or(VchiError::ServiceNotFound(handle))?
            .borrow_mut();

        if let Some(peek_buf) = service.peek_buf.take() {
            info!("using peek buffer for dequeue_message");
            Ok(peek_buf)
        } else {
            let handle = service.handle;
            Arc::clone(&self.vchiq_instance)
                .dequeue_message(handle, max_data_to_read)
                .await
        }
    }
}
