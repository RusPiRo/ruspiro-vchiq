/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Panic Handler
//!
//! Minimalistic panic handler implementation
//!

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Panicing means we have reach a situation we are unable to recover from into a valid state.
    // Halt the panicing core and safely do nothing!
    // TODO: we might be able to hint where the panic occured to a console output or similar in the future
    // Panicing is undefined behaviour so we are unable to recover from one into a valid state.
    // Halt the panicing core and safely do nothing!
    if let Some(location) = info.location() {
        ruspiro_console::error!(
            "PANIC at {:?}, {}:{}",
            location.file(),
            location.line(),
            location.column()
        );
    } else {
        ruspiro_console::error!("PANIC somewhere!");
    }

    loop {}
}

#[lang = "eh_personality"]
fn eh_personality() {
    // for the time beeing - nothing to be done in this function as the usecase is a bit unclear
}
