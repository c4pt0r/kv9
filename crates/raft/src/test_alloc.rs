//! Per-thread allocator measurement for bounded-copy regression tests only.
//! It measures allocations during a synchronous operation, not process RSS.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
struct State {
    current: usize,
    peak: usize,
}
thread_local! { static ACTIVE: Cell<Option<State>> = const { Cell::new(None) }; }
struct Meter;
#[global_allocator]
static ALLOCATOR: Meter = Meter;

fn changed(added: usize, removed: usize) {
    let _ = ACTIVE.try_with(|cell| {
        if let Some(mut state) = cell.get() {
            state.current = state.current.saturating_sub(removed).saturating_add(added);
            state.peak = state.peak.max(state.current);
            cell.set(Some(state));
        }
    });
}
// SAFETY: every operation forwards the unchanged layout/pointer to System.
// Thread-local accounting performs no allocation and cannot unwind.
unsafe impl GlobalAlloc for Meter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the allocator contract's valid layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            changed(layout.size(), 0);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies the allocator contract's valid layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            changed(layout.size(), 0);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        changed(0, layout.size());
        // SAFETY: the pointer and original layout are forwarded unchanged.
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplies the same contracts required by System.
        let next = unsafe { System.realloc(ptr, layout, new_size) };
        if !next.is_null() {
            changed(new_size, layout.size());
        }
        next
    }
}
pub(crate) fn peak<T>(work: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|cell| cell.set(None));
        }
    }
    ACTIVE.with(|cell| {
        assert!(cell.get().is_none());
        cell.set(Some(State::default()));
    });
    let reset = Reset;
    let result = work();
    let bytes = ACTIVE.with(|cell| cell.get().unwrap().peak);
    drop(reset);
    (result, bytes)
}
