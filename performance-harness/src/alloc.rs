use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

pub struct CountingAllocator;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub allocations: u64,
    pub requested_bytes: u64,
    pub peak_live_bytes: u64,
    pub retained_bytes: i64,
}

fn record_alloc(size: usize) {
    let live = LIVE.try_with(|live| {
        let next = live.get() + size as i64;
        live.set(next);
        next
    });
    let Ok(live) = live else {
        return;
    };
    let _ = ENABLED.try_with(|enabled| {
        if enabled.get() {
            ALLOCS.with(|allocs| allocs.set(allocs.get() + 1));
            BYTES.with(|bytes| bytes.set(bytes.get() + size as u64));
            PEAK.with(|peak| peak.set(peak.get().max(live)));
        }
    });
}

fn record_dealloc(size: usize) {
    let _ = LIVE.try_with(|live| live.set(live.get() - size as i64));
}

fn record_realloc(old_size: usize, new_size: usize) {
    let live = LIVE.try_with(|live| {
        let next = live.get() - old_size as i64 + new_size as i64;
        live.set(next);
        next
    });
    let Ok(live) = live else {
        return;
    };
    let _ = ENABLED.try_with(|enabled| {
        if enabled.get() {
            ALLOCS.with(|allocs| allocs.set(allocs.get() + 1));
            BYTES.with(|bytes| bytes.set(bytes.get() + new_size as u64));
            PEAK.with(|peak| peak.set(peak.get().max(live)));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        record_dealloc(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let resized = System.realloc(ptr, layout, new_size);
        if !resized.is_null() {
            record_realloc(layout.size(), new_size);
        }
        resized
    }
}

struct DisableOnDrop;

impl Drop for DisableOnDrop {
    fn drop(&mut self) {
        ENABLED.with(|enabled| enabled.set(false));
    }
}

pub fn profile<T>(f: impl FnOnce() -> T) -> (T, Profile) {
    assert!(
        !ENABLED.with(|enabled| enabled.replace(true)),
        "allocation profiles cannot be nested"
    );
    let _guard = DisableOnDrop;
    ALLOCS.with(|allocs| allocs.set(0));
    BYTES.with(|bytes| bytes.set(0));
    let baseline = LIVE.with(Cell::get);
    PEAK.with(|peak| peak.set(baseline));

    let out = f();
    ENABLED.with(|enabled| enabled.set(false));
    let live = LIVE.with(Cell::get);
    let peak = PEAK.with(Cell::get);
    let result = Profile {
        allocations: ALLOCS.with(Cell::get),
        requested_bytes: BYTES.with(Cell::get),
        peak_live_bytes: peak.saturating_sub(baseline) as u64,
        retained_bytes: live - baseline,
    };
    (out, result)
}

pub fn count<T>(f: impl FnOnce() -> T) -> (T, u64, u64) {
    let (out, profile) = profile(f);
    (out, profile.allocations, profile.requested_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_distinguishes_peak_from_retained_requested_bytes() {
        let ((), released) = profile(|| {
            let bytes = vec![7_u8; 4_096];
            std::hint::black_box(bytes.as_ptr());
            drop(bytes);
        });
        assert!(released.allocations >= 1);
        assert!(released.requested_bytes >= 4_096);
        assert!(released.peak_live_bytes >= 4_096);
        assert_eq!(released.retained_bytes, 0);

        let (retained, live) = profile(|| vec![9_u8; 4_096].into_boxed_slice());
        assert!(live.retained_bytes >= 4_096);
        assert!(live.peak_live_bytes >= live.retained_bytes as u64);
        drop(retained);
    }
}
