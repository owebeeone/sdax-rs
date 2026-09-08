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

pub fn write_csv(samples: &[Sample]) {
    println!("phase,workload,nodes,edges,sample,nanos,allocations,allocated_bytes,checksum");
    for s in samples {
        println!(
            "{},{},{},{},{},{},{},{},{}",
            s.phase,
            s.workload,
            s.nodes,
            s.edges,
            s.sample,
            s.nanos,
            s.allocations,
            s.allocated_bytes,
            s.checksum
        );
    }
}
