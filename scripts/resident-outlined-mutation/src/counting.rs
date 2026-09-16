//! Request counters only. This module is absent from the latency executable.
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

pub struct Allocator;
static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);
thread_local! { static WINDOW: Cell<Option<[u64; 6]>> = const { Cell::new(None) }; }

fn change_live(delta: i64) {
    let current = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
    PEAK.fetch_max(current, Ordering::Relaxed);
}
fn record(slot: usize, bytes: usize) {
    WINDOW.with(|window| {
        if let Some(mut counts) = window.get() {
            counts[slot] += 1;
            counts[slot + 1] += bytes as u64;
            window.set(Some(counts));
        }
    });
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { tikv_jemallocator::Jemalloc.alloc(layout) };
        if !pointer.is_null() {
            change_live(layout.size() as i64);
            record(0, layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { tikv_jemallocator::Jemalloc.alloc_zeroed(layout) };
        if !pointer.is_null() {
            change_live(layout.size() as i64);
            record(0, layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record(4, layout.size());
        change_live(-(layout.size() as i64));
        unsafe { tikv_jemallocator::Jemalloc.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { tikv_jemallocator::Jemalloc.realloc(pointer, layout, size) };
        if !next.is_null() {
            record(2, size);
            change_live(size as i64 - layout.size() as i64);
        }
        next
    }
}
pub fn live() -> i64 {
    LIVE.load(Ordering::Relaxed)
}
pub fn peak() -> i64 {
    PEAK.load(Ordering::Relaxed)
}
pub fn start() {
    PEAK.store(live(), Ordering::Relaxed);
    WINDOW.with(|window| {
        assert!(window.get().is_none());
        window.set(Some([0; 6]));
    });
}
pub fn stop() -> [u64; 6] {
    WINDOW.with(|window| window.replace(None).unwrap())
}
