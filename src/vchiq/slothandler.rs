/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SlotHandler
//!
use super::{
    config::*,
    service::{Service, ServiceState, VCHIQ_PORT_FREE},
    shared::{
        slot::{slot_queue_index_from_pos, SlotPosition},
        slotmessage::{
            calc_msg_stride, dst_port_from_id, msg_type_from_id, src_port_from_id, MessageType,
            OpenAckPayload,
        },
    },
    state::State,
};
use crate::types::Reason;
use alloc::{boxed::Box, sync::Arc};
use core::{
    convert::TryInto,
    future::{Future, Pending},
    pin::Pin,
    task::{Context, Poll},
};
use ruspiro_console::*;
use ruspiro_lock::r#async::AsyncMutexGuard;

pub(crate) fn slot_handler(state: Arc<State>, cx: &mut Context<'_>) -> Poll<()> {
    if let Some(mut slot_state) = state.slot_state.try_lock() {
        let tx_pos = slot_state.slot_zero.remote_ref().tx_pos() as usize;

        while slot_state.rx_pos != tx_pos {
            // if the states rx data does not point to anything means there is no open previous messages that needs
            // to be processed further with this package
            if slot_state.rx_data.is_none() {
                let slot_queue_index =
                    slot_queue_index_from_pos(tx_pos as usize) as usize & VCHIQ_SLOT_QUEUE_MASK;
                info!("slot_queue_index {}", slot_queue_index);
                let slot_index =
                    slot_state.slot_zero.remote_ref().slot_queue()[slot_queue_index] as usize;
                info!("slot_index {}", slot_index);
                slot_state.rx_data.replace(SlotPosition::new(slot_index, 0));
                //state.rx_info = state.slot_info_from_index(slot_index);
                /*if let Some(slot_info) = state.slot_infos.get_mut(slot_index as usize) {
                    (*slot_info).use_count = 1;
                    (*slot_info).release_count = 0;
                }*/
            }

            info!("handle message from {:?}", slot_state.rx_data);
            if let Some(rx_data) = slot_state.rx_data {
                let slot_data = slot_state.slot_data.read_message(rx_data);

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
                        let service = state.service_by_port(local_port);
                        if let Some(service) = &service {
                            if service.borrow().remoteport != remote_port
                                && service.borrow().remoteport != VCHIQ_PORT_FREE
                                && local_port == 0
                            {
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
                    }
                    MessageType::OPENACK
                    | MessageType::DATA
                    | MessageType::BULK_RX
                    | MessageType::BULK_TX
                    | MessageType::BULK_RX_DONE
                    | MessageType::BULK_TX_DONE => {
                        let service = state.service_by_port(local_port);
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
                    }
                    MessageType::CLOSE => {
                        if size > 0 {
                            warn!("close should not contain any data");
                        }
                        let service = service.expect("CLOSE expects a service to be available");
                        service.borrow_mut().closing = true;
                        let quota_state = state.quota_state.read();
                        quota_state.service_quotas[service.borrow().localport as usize]
                            .quota_event
                            .up();

                        // the following part is like close_service_internal called with close_recvd = true
                        let srvstate = service.borrow().srvstate;
                        info!("close service with state {:?}", srvstate);
                        match srvstate {
                            ServiceState::CLOSED
                            | ServiceState::HIDDEN
                            | ServiceState::LISTENING
                            | ServiceState::CLOSEWAIT => {
                                error!("closing service called in state {:?}", srvstate);
                            }
                            ServiceState::OPENING => {
                                service.borrow_mut().set_state(ServiceState::CLOSEWAIT);
                                service.borrow().remove_event.up();
                            }
                            ServiceState::CLOSESENT => {
                                // TODO: do_abort_bulks

                                let _ = state.close_service_complete(
                                    Arc::clone(&service),
                                    ServiceState::CLOSERECVD,
                                );
                                // close_service_complete(service, ServiceState::CLOSERECVD);
                            }
                            ServiceState::CLOSERECVD => {}
                            _ => warn!("close recieved for service in state {:?}", srvstate),
                        }
                    }
                    MessageType::DATA => {
                        let service = service.expect("DATA expects a service to be available");
                        info!(
                            "DATA Message local/remote {}/{} - srv.local/remote {}/{}",
                            local_port,
                            remote_port,
                            service.borrow().localport,
                            service.borrow().remoteport,
                        );
                        if service.borrow().remoteport == remote_port
                            && service.borrow().srvstate == ServiceState::OPEN
                        {
                            // header->msg_id = msgid | MSGID_CLAIMED;
                            // claim_slot(slot_state.rx_info);
                            // now invoke the service callback passing the actuall received data to it
                            info!(
                                "data received for service, invoke callback with {:?}",
                                slot_data
                            );
                            let service_clone = Arc::clone(&service);
                            let callback = service.borrow_mut().base.callback.take();
                            let srv_handle = service.borrow().handle;
                            if let Some(callback) = callback {
                                (callback)(
                                    Reason::MESSAGE_AVAILABLE,
                                    Some(Arc::new(slot_data)),
                                    srv_handle,
                                    None,
                                );
                                service.borrow_mut().base.callback.replace(callback);
                            };
                        }
                    }
                    MessageType::CONNECT => {
                        info!(
                            "CONNECT - version common {}",
                            slot_state.slot_zero.version()
                        );
                        // set the connect semaphore which is checked while connecting
                        // to the VCHIQ - see VchiqInstance::connect()
                        state.connect.up();
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

                slot_state.rx_pos += msg_stride;

                if (slot_state.rx_pos as usize & VCHIQ_SLOT_MASK) == 0 {
                    // release_slot()
                    let _ = slot_state.rx_data.take();
                } else {
                    slot_state.rx_data.replace(SlotPosition::new(idx, offset));
                }
                info!("new rx_pos {:?}", slot_state.rx_pos);
            }
        }
    }

    // at the end always wake th waker passed to this function and return Pending
    // as this shall be processed as long as the VCHIQ is alive
    cx.waker().wake_by_ref();
    Poll::Pending
}

/*
struct SlotHandler {
    state: Arc<State>,
}

impl Future for SlotHandler {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SlotHandler processing is checking for the local trigger event beeing raised and if this is
        // the case performing the necessary actions.
        // we typically would like to wake this Future in case it makes sense to progress further
        // but for the time beeing we just keep polling and always wake ourself. At the end of the day this
        // Future may never come to a conclusion as it need to wait for new things to happen until the VCHIQ
        // interface is stopped (which actually only happens if the Raspberry Pi is switched off completely)
        //info!("poll handle slots");
        match Box::pin(self.state.handle_slots()).as_mut().poll(cx) {
            Poll::Pending => Poll::Pending, // the called future will wake this Future...
            Poll::Ready(_) => {
                // even though we are ready we are required to be polled again
                // TODO: check for an optimized waking strategy
                info!("wake slothandler by ref");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
        //info!("handle slots done");
        /*
        if let Poll::Ready(mut state) = Box::pin(self.state.lock()).as_mut().poll(cx) {
            //info!("SlotHandler locked State");
            //info!("Local State {:#?}", state.slot_zero.local_ref());
            let trigger_waiter = state.slot_zero_mut().local_mut().trigger_mut().wait();
            let waiter_result = Box::pin(trigger_waiter).as_mut().poll(cx);
            if let Poll::Ready(_) = waiter_result {
                info!("local trigger event recognised");
                self.parse_rx_slots(state);
            }
        }
        */
    }
}
*/
