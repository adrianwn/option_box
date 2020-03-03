// Copyright 2020 Adrian Willenbücher
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use std::marker::PhantomData;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

pub struct OptionArc<T> {
    ptr: AtomicPtr<T>,
    phantom: PhantomData<Option<Arc<T>>>,
}

impl<T> OptionArc<T> {
    pub fn new() -> OptionArc<T> {
        OptionArc {
            ptr: AtomicPtr::new(null_mut()),
            phantom: PhantomData,
        }
    }

    pub fn into_inner(mut v: OptionArc<T>) -> Option<Arc<T>> {
        let raw: *mut T = *v.ptr.get_mut();
        std::mem::forget(v);
        if raw.is_null() {
            None
        } else {
            Some(unsafe { Arc::from_raw(raw) })
        }
    }

    pub fn set(&self, v: Arc<T>) {
        let raw = Arc::into_raw(v);
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
            drop(unsafe { Arc::from_raw(raw) });
            panic!("OptionArc has already been set");
        }
    }
}

impl<T> std::ops::Deref for OptionArc<T> {
    type Target = T;

    fn deref(&self) -> &T {
        let raw = self.ptr.load(Ordering::Acquire);
        assert!(!raw.is_null(), "OptionArc<T> has not been set yet");
        unsafe { &*raw }
    }
}

impl<T> Clone for OptionArc<T> {
    fn clone(&self) -> OptionArc<T> {
        let raw = self.ptr.load(Ordering::Acquire);
        let new_raw = if raw.is_null() {
            null()
        } else {
            let arc = std::mem::ManuallyDrop::new(unsafe { Arc::from_raw(raw) });
            Arc::into_raw((*arc).clone())
        };
        OptionArc {
            ptr: AtomicPtr::new(new_raw as *mut _),
            phantom: PhantomData,
        }
    }
}

impl<T> Drop for OptionArc<T> {
    fn drop(&mut self) {
        // No need for atomics because we have a &mut reference.
        let raw: *mut T = *self.ptr.get_mut();
        if !raw.is_null() {
            drop(unsafe { Arc::from_raw(raw) });
        }
    }
}

impl<T> From<Option<Arc<T>>> for OptionArc<T> {
    fn from(v: Option<Arc<T>>) -> OptionArc<T> {
        let raw = match v {
            Some(arc) => Arc::into_raw(arc),
            None => null(),
        };
        OptionArc {
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

    struct Indicator {
        value: Cell<u32>,
        drop_ctr: *const AtomicUsize,
    }

    impl Drop for Indicator {
        fn drop(&mut self) {
            unsafe { (*self.drop_ctr).fetch_add(1, Ordering::SeqCst); }
        }
    }

    #[test]
    fn set_1() {
        let drop_ctr = AtomicUsize::new(0);
        let arc = Arc::new(Indicator {
            value: Cell::new(12345),
            drop_ctr: &drop_ctr as *const _,
        });
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);

        let b1: OptionArc<Indicator> = OptionArc::new();
        b1.set(arc.clone());
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        assert_eq!(b1.value.get(), 12345);
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        drop(b1);
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        drop(arc);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    fn set_2() {
        let drop_ctr = AtomicUsize::new(0);
        let arc = Arc::new(Indicator {
            value: Cell::new(123456),
            drop_ctr: &drop_ctr as *const _,
        });
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);

        let b1: OptionArc<Indicator> = OptionArc::new();
        b1.set(arc);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        assert_eq!(b1.value.get(), 123456);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        drop(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    #[should_panic]
    fn set_twice() {
        let drop_ctr = AtomicUsize::new(0);
        let b1: OptionArc<Indicator> = OptionArc::new();
        b1.set(Arc::new(Indicator {
            value: Cell::new(5),
            drop_ctr: &drop_ctr as *const _,
        }));
        b1.set(Arc::new(Indicator {
            value: Cell::new(6),
            drop_ctr: &drop_ctr as *const _,
        }));
    }

    #[test]
    #[should_panic]
    fn deref_unset() {
        let b1: OptionArc<Indicator> = OptionArc::new();
        let _ = *b1;
    }

    #[test]
    fn into_inner() {
        let drop_ctr = AtomicUsize::new(0);
        let arc = Arc::new(Indicator {
            value: Cell::new(23456),
            drop_ctr: &drop_ctr as *const _,
        });
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);

        let b1: OptionArc<Indicator> = OptionArc::new();
        b1.set(arc.clone());
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        let opt_inner: Option<Arc<Indicator>> = OptionArc::into_inner(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);

        let inner = opt_inner.unwrap();
        assert_eq!(inner.value.get(), 23456);
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);

        drop(inner);
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
    }

    #[test]
    fn into_inner_unset() {
        let b1: OptionArc<Indicator> = OptionArc::new();
        let opt_inner: Option<Arc<Indicator>> = OptionArc::into_inner(b1);
        assert!(opt_inner.is_none());
    }

    #[test]
    fn clone() {
        let drop_ctr = AtomicUsize::new(0);
        let arc = Arc::new(Indicator {
            value: Cell::new(15),
            drop_ctr: &drop_ctr as *const _,
        });

        let b1: OptionArc<Indicator> = OptionArc::new();
        b1.set(arc.clone());
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);

        let b2 = b1.clone();
        assert_eq!(Arc::strong_count(&arc), 3);
        assert_eq!(Arc::weak_count(&arc), 0);

        b2.value.set(16);
        assert_eq!(b1.value.get(), 16);
        assert_eq!(b2.value.get(), 16);

        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&arc), 3);
        assert_eq!(Arc::weak_count(&arc), 0);

        drop(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&arc), 2);
        assert_eq!(Arc::weak_count(&arc), 0);

        drop(b2);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);
        assert_eq!(Arc::strong_count(&arc), 1);
        assert_eq!(Arc::weak_count(&arc), 0);

        drop(arc);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    fn from_some() {
        let drop_ctr = AtomicUsize::new(0);
        let arc = Arc::new(Indicator {
            value: Cell::new(34567),
            drop_ctr: &drop_ctr as *const _,
        });

        let v: Option<Arc<Indicator>> = Some(arc);
        let b1: OptionArc<Indicator> = From::from(v);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 0);

        assert_eq!(b1.value.get(), 34567);
        drop(b1);
        assert_eq!(drop_ctr.load(Ordering::Acquire), 1);
    }

    #[test]
    fn from_none() {
        let v: Option<Arc<Indicator>> = None;
        let b1: OptionArc<Indicator> = From::from(v);
        assert!(OptionArc::into_inner(b1).is_none());
    }
}
