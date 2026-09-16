//! Regression controls for ownership restoration, including panic after replacement.
use super::ErasedPtr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc as CounterOwner;

#[derive(Debug)]
struct Tracked {
    id: usize,
    drops: CounterOwner<[AtomicUsize; 2]>,
    panic_clone: bool,
}
impl Clone for Tracked {
    fn clone(&self) -> Self {
        assert!(!self.panic_clone, "intentional clone failure");
        Self { id: self.id, drops: self.drops.clone(), panic_clone: false }
    }
}
impl Drop for Tracked {
    fn drop(&mut self) { self.drops[self.id].fetch_add(1, Ordering::SeqCst); }
}
macro_rules! pointer_cases {
    ($name:ident, $pointer:path) => {
        mod $name {
            use super::*;
            use $pointer as Pointer;
            use std::cell::Cell;
            use std::panic::{catch_unwind, AssertUnwindSafe};

            fn replacement(panic_after: bool) {
                let drops = CounterOwner::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
                let original = Pointer::new(Tracked { id: 0, drops: drops.clone(), panic_clone: false });
                let old_view = original.clone();
                let mut slot = ErasedPtr::new(Pointer::into_raw(original));
                let expected = Cell::new(core::ptr::null());
                let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                    slot.map_owned::<Tracked, Pointer<Tracked>, _>(Pointer::from_raw, Pointer::as_ptr, |owned| {
                        *owned = Pointer::new(Tracked { id: 1, drops: drops.clone(), panic_clone: false });
                        expected.set(Pointer::as_ptr(owned));
                        assert!(!panic_after, "intentional failure after pointer replacement");
                        17
                    })
                }));
                assert_eq!(result.is_err(), panic_after);
                if !panic_after { assert_eq!(result.unwrap(), 17); }
                // Compare addresses before any dereference/reconstruction; a stale-slot mutant
                // fails safely while old_view still keeps the old allocation alive.
                assert_eq!(unsafe { slot.cast::<Tracked>() }, expected.get());
                assert_eq!(Pointer::strong_count(&old_view), 1);
                assert_eq!(old_view.id, 0);
                assert_eq!(drops[0].load(Ordering::SeqCst), 0);
                assert_eq!(drops[1].load(Ordering::SeqCst), 0);
                let current = unsafe { Pointer::from_raw(slot.cast::<Tracked>()) };
                assert_eq!(current.id, 1);
                assert_eq!(Pointer::strong_count(&current), 1);
                drop(current);
                assert_eq!(drops[1].load(Ordering::SeqCst), 1);
                drop(old_view);
                assert_eq!(drops[0].load(Ordering::SeqCst), 1);
            }
            #[test]
            fn normal_replacement_preserves_old_view_and_releases_both_once() { replacement(false); }
            #[test]
            fn panic_after_replacement_restores_current_pointer_and_owned_reference() { replacement(true); }
            #[test]
            fn clone_panic_preserves_both_references_and_releases_value_once() {
                let drops = CounterOwner::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
                let original = Pointer::new(Tracked { id: 0, drops: drops.clone(), panic_clone: true });
                let old_view = original.clone();
                let address = Pointer::as_ptr(&original);
                let mut slot = ErasedPtr::new(Pointer::into_raw(original));
                let result = catch_unwind(AssertUnwindSafe(|| unsafe {
                    slot.map_owned::<Tracked, Pointer<Tracked>, _>(Pointer::from_raw, Pointer::as_ptr, |owned| {
                        Pointer::make_mut(owned).id = 1;
                    })
                }));
                assert!(result.is_err());
                assert_eq!(unsafe { slot.cast::<Tracked>() }, address);
                assert_eq!(Pointer::strong_count(&old_view), 2);
                assert_eq!(old_view.id, 0);
                assert_eq!(drops[0].load(Ordering::SeqCst), 0);
                let current = unsafe { Pointer::from_raw(slot.cast::<Tracked>()) };
                drop(current);
                assert_eq!(Pointer::strong_count(&old_view), 1);
                assert_eq!(drops[0].load(Ordering::SeqCst), 0);
                drop(old_view);
                assert_eq!(drops[0].load(Ordering::SeqCst), 1);
                assert_eq!(drops[1].load(Ordering::SeqCst), 0);
            }
        }
    };
}
pointer_cases!(rc, std::rc::Rc);
pointer_cases!(arc, std::sync::Arc);
#[cfg(feature = "triomphe")]
pointer_cases!(triomphe_arc, triomphe::Arc);
