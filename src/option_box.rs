// Copyright 2020 Adrian Willenbücher
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use std::marker::PhantomData;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct OptionBox<T> {
    ptr: AtomicPtr<T>,
    phantom: PhantomData<Option<Box<T>>>,
}

impl<T> OptionBox<T> {
    pub fn new() -> OptionBox<T> {
        OptionBox {
            ptr: AtomicPtr::new(null_mut()),
            phantom: PhantomData,
        }
    }

    pub fn into_inner(mut v: OptionBox<T>) -> Option<Box<T>> {
        let raw: *mut T = *v.ptr.get_mut();
        std::mem::forget(v);
        if raw.is_null() {
            None
        } else {
            Some(unsafe { Box::from_raw(raw) })
        }
    }

    pub fn set(&self, v: Box<T>) {
        let raw = Box::into_raw(v);
        // Success ordering is Release so that a subsequent deref/drop creates a
        // Release-Acquire pair.
        // Failure ordering is Relaxed, because in that case we don't do anything
        // with the current value of self.ptr.
        if self.ptr.compare_exchange(
            null_mut(),
            raw as *mut _,
            Ordering::Release,
            Ordering::Relaxed,
        ).is_err() {
            drop(unsafe { Box::from_raw(raw) });
            panic!("OptionBox has already been set");
        }
    }
}

impl<T> std::ops::Deref for OptionBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let raw = self.ptr.load(Ordering::Acquire);
        assert!(!raw.is_null(), "OptionBox<T> has not been set yet");
        unsafe { &*raw }
    }
}

impl<T: Clone> Clone for OptionBox<T> {
    fn clone(&self) -> OptionBox<T> {
        let raw = self.ptr.load(Ordering::Acquire);
        let new_raw = if raw.is_null() {
            null()
        } else {
            let b = std::mem::ManuallyDrop::new(unsafe { Box::from_raw(raw) });
            Box::into_raw((*b).clone())
        };
        OptionBox {
            ptr: AtomicPtr::new(new_raw as *mut _),
            phantom: PhantomData,
        }
    }
}

impl<T> Drop for OptionBox<T> {
    fn drop(&mut self) {
        // No need for atomics because we have a &mut reference.
        let raw: *mut T = *self.ptr.get_mut();
        if !raw.is_null() {
            drop(unsafe { Box::from_raw(raw) });
        }
    }
}

impl<T> From<Option<Box<T>>> for OptionBox<T> {
    fn from(v: Option<Box<T>>) -> OptionBox<T> {
        let raw = match v {
            Some(b) => Box::into_raw(b),
            None => null(),
        };
        OptionBox {
            ptr: AtomicPtr::new(raw as *mut _),
            phantom: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone)]
    struct Indicator {
        value: Cell<u32>,
        drop_ctr: Cell<*const AtomicUsize>,
    }

    impl Drop for Indicator {
        fn drop(&mut self) {
            unsafe { (*self.drop_ctr.get()).fetch_add(1, Ordering::SeqCst); }
        }
    }

    #[test]
    fn set() {
        let drop_ctr = AtomicUsize::new(0);
        let b1: OptionBox<Indicator> = OptionBox::new();
        b1.set(Box::new(Indicator {
            value: Cell::new(12345),
            drop_ctr: Cell::new(&drop_ctr as *const _),
        }));
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(b1.value.get(), 12345);
        drop(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    #[should_panic]
    fn set_twice() {
        let drop_ctr = AtomicUsize::new(0);
        let b1: OptionBox<Indicator> = OptionBox::new();
        b1.set(Box::new(Indicator {
            value: Cell::new(5),
            drop_ctr: Cell::new(&drop_ctr as *const _),
        }));
        b1.set(Box::new(Indicator {
            value: Cell::new(6),
            drop_ctr: Cell::new(&drop_ctr as *const _),
        }));
    }

    #[test]
    #[should_panic]
    fn deref_unset() {
        let b1: OptionBox<Indicator> = OptionBox::new();
        let _ = *b1;
    }

    #[test]
    fn into_inner() {
        let drop_ctr = AtomicUsize::new(0);
        let b1: OptionBox<Indicator> = OptionBox::new();
        b1.set(Box::new(Indicator {
            value: Cell::new(23456),
            drop_ctr: Cell::new(&drop_ctr as *const _),
        }));
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        let opt_inner: Option<Box<Indicator>> = OptionBox::into_inner(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        let inner = opt_inner.unwrap();
        assert_eq!(inner.value.get(), 23456);
        drop(inner);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    fn into_inner_unset() {
        let b1: OptionBox<Indicator> = OptionBox::new();
        let opt_inner: Option<Box<Indicator>> = OptionBox::into_inner(b1);
        assert!(opt_inner.is_none());
    }

    #[test]
    fn clone() {
        let drop_ctr_1 = AtomicUsize::new(0);
        let drop_ctr_2 = AtomicUsize::new(0);
        let b1: OptionBox<Indicator> = OptionBox::new();
        b1.set(Box::new(Indicator {
            value: Cell::new(15),
            drop_ctr: Cell::new(&drop_ctr_1 as *const _),
        }));
        let b2 = b1.clone();
        b2.drop_ctr.set(&drop_ctr_2 as *const _);
        b2.value.set(16);
        assert_eq!(b1.value.get(), 15);
        assert_eq!(b2.value.get(), 16);
        assert_eq!(drop_ctr_1.load(Ordering::Acquire), 0);
        assert_eq!(drop_ctr_2.load(Ordering::Acquire), 0);
        drop(b1);
        assert_eq!(drop_ctr_1.load(Ordering::Acquire), 1);
        assert_eq!(drop_ctr_2.load(Ordering::Acquire), 0);
        drop(b2);
        assert_eq!(drop_ctr_1.load(Ordering::Acquire), 1);
        assert_eq!(drop_ctr_2.load(Ordering::Acquire), 1);
    }

    #[test]
    fn from_some() {
        let drop_ctr = AtomicUsize::new(0);
        let v: Option<Box<Indicator>> = Some(Box::new(Indicator {
            value: Cell::new(34567),
            drop_ctr: Cell::new(&drop_ctr as *const _),
        }));
        let b1: OptionBox<Indicator> = From::from(v);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(b1.value.get(), 34567);
        drop(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    fn from_none() {
        let v: Option<Box<Indicator>> = None;
        let b1: OptionBox<Indicator> = From::from(v);
        assert!(OptionBox::into_inner(b1).is_none());
    }
}
