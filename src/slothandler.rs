/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ SlotHandler
//!
use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use super::config::*;
use super::shared::slot::{slot_queue_index_from_pos, SlotPosition};
use super::shared::slotmessage::{calc_msg_stride, msg_type_from_id, MessageType};
use super::state::State;
use ruspiro_console::info;
use ruspiro_lock::r#async::AsyncMutexGuard;

pub async fn slot_handler(state: Arc<State>) {
    SlotHandler { state }.await
}

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

/*

impl SlotHandler {
    fn parse_rx_slots(&self, mut state: AsyncMutexGuard<'_, StateInner>) {
        let tx_pos = state.slot_zero().remote_ref().tx_pos() as u32;
        //info!("current rx_pos {:?} - remote tx_pos {:?}", state.rx_pos, tx_pos);

        while state.rx_pos() != tx_pos {
            // if the states rx data does not point to anything means there is no open previous messages that needs
            // to be processed further with this package
            if state.rx_data().is_none() {
                let slot_queue_index =
                    slot_queue_index_from_pos(tx_pos as usize) as usize & VCHIQ_SLOT_QUEUE_MASK;
                info!("slot_queue_index {}", slot_queue_index);
                let slot_index =
                    state.slot_zero().remote_ref().slot_queue()[slot_queue_index] as usize;
                info!("slot_index {}", slot_index);
                state
                    .rx_data_mut()
                    .replace(SlotPosition::new(slot_index, 0));
                //state.rx_info = state.slot_info_from_index(slot_index);
                /*if let Some(slot_info) = state.slot_infos.get_mut(slot_index as usize) {
                    (*slot_info).use_count = 1;
                    (*slot_info).release_count = 0;
                }*/
            }

            info!("handle message from {:?}", state.rx_data());
            if let Some(ref rx_data) = state.rx_data() {
                let slot_data = state.slot_data().read_message(*rx_data);

                let msg_type = msg_type_from_id(slot_data.header.msgid);
                let size = slot_data.header.size;
                info!(
                    "slot header - message type {:#?} / size {:#x?}",
                    msg_type, size
                );

                match msg_type {
                    MessageType::OPENACK
                    | MessageType::CLOSE
                    | MessageType::DATA
                    | MessageType::BULK_RX
                    | MessageType::BULK_TX
                    | MessageType::BULK_RX_DONE
                    | MessageType::BULK_TX_DONE => {
                        unimplemented!();
                    }
                    _ => (),
                }

                match msg_type {
                    MessageType::OPEN => unimplemented!(),
                    MessageType::OPENACK => unimplemented!(),
                    MessageType::CLOSE => unimplemented!(),
                    MessageType::DATA => unimplemented!(),
                    MessageType::CONNECT => {
                        info!("version common {}", state.slot_zero().version());
                        // set the connect semaphore which is checked while connecting
                        // to the VCHIQ - see VchiqInstance::connect()
                        state.connect().up();
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
                let rx_pos = state.rx_pos();

                state.set_rx_pos(rx_pos + msg_stride as u32);

                if (state.rx_pos() as usize & VCHIQ_SLOT_MASK) == 0 {
                    // release_slot()
                    let _ = state.rx_data_mut().take();
                } else {
                    state.rx_data_mut().replace(SlotPosition::new(idx, offset));
                }
                info!("new rx_pos {:?}", state.rx_pos());
            }
        }
    }
}
*/
