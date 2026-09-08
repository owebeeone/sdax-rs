use crate::alloc;
use std::hint::black_box;
use std::time::Instant;

#[derive(Clone)]
pub struct Sample {
    pub phase: &'static str,
    pub workload: String,
    pub nodes: usize,
    pub edges: usize,
    pub sample: usize,
    pub nanos: u128,
    pub allocations: u64,
    pub allocated_bytes: u64,
    pub peak_live_bytes: u64,
    pub retained_bytes: i64,
    pub checksum: u64,
}

pub fn timed<T>(f: impl FnOnce() -> (T, u64)) -> (u128, u64) {
    let started = Instant::now();
    let (value, checksum) = f();
    black_box(value);
    (started.elapsed().as_nanos(), black_box(checksum))
}

pub fn allocated<T>(f: impl FnOnce() -> (T, u64)) -> (u64, u64, u64) {
    let ((value, checksum), allocations, bytes) = alloc::count(f);
    black_box(value);
    (allocations, bytes, black_box(checksum))
}

pub fn profiled<T>(f: impl FnOnce() -> (T, u64)) -> (alloc::Profile, u64) {
    let ((value, checksum), profile) = alloc::profile(f);
    black_box(value);
    (profile, black_box(checksum))
}

pub fn write_csv(samples: &[Sample]) {
    println!(
        "phase,workload,nodes,edges,sample,nanos,allocations,allocated_bytes,peak_live_bytes,retained_bytes,checksum"
    );
    for s in samples {
        println!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            s.phase,
            s.workload,
            s.nodes,
            s.edges,
            s.sample,
            s.nanos,
            s.allocations,
            s.allocated_bytes,
            s.peak_live_bytes,
            s.retained_bytes,
            s.checksum
        );
    }
}
