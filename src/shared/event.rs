/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # VCHIQ Event
//!

use alloc::format;
use core::fmt;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use ruspiro_lock::sync::Semaphore;
use crate::doorbell::ring_vc_doorbell;
use ruspiro_arch_aarch64::instructions::*;
use ruspiro_console::*;

/// Representation of a shared event that may be raised either from ARM or from VC side
#[derive(Debug, Default)]
#[repr(C)]
pub struct Event {
    /// Flag that is either 1 or 0 indicating the event is "armed". This is typically set from the receiver side
    /// handling the event once fired.
    armed: u32,
    /// Flag that is either 1 or 0 indicating the event has been "fired". This is typically set from the sender side
    /// requesting the receiver to handle this event.
    fired: u32,
    /// This is a raw pointer to a semaphore on either side used to trigger threads processing an event that has been
    /// fired. This raw pointer is only valid on the respective owning side. An event owned by the VideoCore will put
    /// its local memory address into this field. As the VC firmware uses only 32Bit wide pointers this field need to
    /// also be 32Bit on ARM side regardless of the possibility to use 64Bit pointers on ARM. The events owned by the
    /// ARM side will manage their semaphores linked to this event separately as storage of 64Bit pointer into 32Bit
    /// variable is error prone event if the effective address space used on the Raspberry PI might not need the upper
    /// 32Bits to be used.
    event: u32,
}

pub struct LocalEvent;
pub struct RemoteEvent;

/// As the [Event] data is shared between the ARM and the VideoCore special care need to be taken when reading
/// data from the memory location of this data or writing to the same. Rust should never be allowed to optimize such
/// access which will happen as the ARM side might only see writes to the data but no reads, which could lead to an
/// optimization, that removes all actual writes to the data and hence to undefined behavior.
/// So this accessor struct is the only way to read and update data from an [Event].
pub struct EventAccessor<T> {
    inner: *mut Event,
    /// explicitely handled Semaphore for the [Event] that runs in aarch32 and aarch64 build targets.
    /// this is optional as it is only applicable for events owned by the ARM side
    event: Option<Semaphore>,
    _type: core::marker::PhantomData<T>,
}

impl<T> EventAccessor<T> {
    /// Create a new [EventAccessor] based on the memory address of the [Event]
    /// # Safety
    /// This is safe if it is guaranteed that the raw pointer actually points to a location that is an [Event] and the
    /// lifetime of this raw pointer outlives the accessor.
    pub unsafe fn new(event: *mut Event) -> Self {
        Self {
            inner: event,
            event: None,
            _type: core::marker::PhantomData,
        }
    }

    volatile_getter!(armed, u32);
    volatile_setter!(armed, u32);

    volatile_getter!(fired, u32);
    volatile_setter!(fired, u32);

    volatile_getter!(event, u32);

    /// Initialize the Event. If it is owned by the ARM side, the respective [Semaphore] should be passed to allow
    /// notification / waking of async [Future]s waiting to make progress.
    pub fn init(&mut self, event: Option<Semaphore>) {
        self.set_armed(0);
        if let Some(event) = event {
            self.event.replace(event);
        }
    }
}

impl EventAccessor<LocalEvent> {
    /// Raise the local semaphore linked with this event
    pub fn signal_local(&mut self) {
        self.set_armed(0);
        if let Some(ref event) = self.event {
            event.up();
        }
    }

    pub async fn wait(&mut self) {
        self.set_armed(1);
        dsb();
        self.await
    }
}

impl Future for EventAccessor<LocalEvent> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // check if the event has been fired. We need memory barriers to ensure actual data is read
        unsafe {
            let accessor = self.get_mut();//unchecked_mut();
            dmb();
            if accessor.fired() == 0 {
                // it's not yet fired, so "arm" it to signal to the other side we would be ready for immediate handling
                accessor.set_armed(1);
                // memory barrier to ensure data operation is finished before continuing
                dsb();
                // check if the other side now fired immediately
                if accessor.fired() == 0 {
                    // no - not fired so check for the semaphore assigned to the event if it was raised in the meantime
                    // TODO: check how this could used to wake this Future once the semaphore is raised...
                    //       as the semaphore is very likely only being raised if the corresponding doorbell IRQ is
                    //       received, we might attach the waker to the IRQ handler directly and skip the semaphore at
                    //       all, if the IRQ was recieved the event state if triggered would reflect "fired" == 1
                    if let Some(ref event) = accessor.event {
                        if event.try_down().is_err() {
                            // reset the armed state and keep waiting
                            //accessor.set_armed(0);
                            // immediately wake the Future to recheck the event - TODO: optimize to only wake if
                            // semaphore was raised
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        } else {
                            // reset armed state and finish waiting
                            accessor.set_armed(0);
                            dmb();
                            return Poll::Ready(());
                        }
                    } else {
                        panic!("called wait on event not owned by ARM!");
                    }
                }
            }
            // getting here means we were able to read "fired" != 0
            // reset the fired state as we have handled the same and stop waiting
            accessor.set_fired(0);
            return Poll::Ready(());
        }
    }
}

impl EventAccessor<RemoteEvent> {
    /// Set the event to "fired" and let the other side (VideoCore) know that there is an event that needs attention.
    pub fn signal_remote(&mut self) {
        dmb();
        // fire the event
        self.set_fired(1);
        dsb();
        // if the other side was not able to immediately pick the event up
        // ring the doorbell to let it know the event state has changed and needs attention
        if self.armed() != 0 {
            ring_vc_doorbell();
        }
    }
}

impl<T> fmt::Debug for EventAccessor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let event = if self.event.is_some() {
            format!("{:#?}", self.event)
        } else {
            let event = self.event();
            format!("{:#x?}", event)
        };

        f.debug_struct("Event")
            .field("armed", &self.armed())
            .field("fired", &self.fired())
            .field("event", &event)
            .finish()
    }
}
