/***********************************************************************************************************************
 * Copyright (c) 2019 by the authors
 *
 * Author: André Borrmann
 * License: Apache License 2.0
 **********************************************************************************************************************/

//! # Helper Macros

/// macro to help with the implementation of volatile getter for the different fields of the [SlotZero] structure
macro_rules! volatile_getter {
    ($field:ident, $t:ty) => {
        #[allow(dead_code)]
        pub fn $field(&self) -> $t {
            unsafe { core::ptr::read_volatile(&((*self.inner).$field)) }
        }
    };
}

/// macro to help with the implementation of volatile getter for the different fields of the [SlotZero] structure
macro_rules! volatile_setter {
    ($field:ident, $t:ty) => {
        paste::item! {
            #[allow(dead_code)]
            pub fn [<set_ $field>](&mut self, value: $t) {
                unsafe {
                    core::ptr::write_volatile(&mut ((*self.inner).$field), value);
                }
            }
        }
    };
}
