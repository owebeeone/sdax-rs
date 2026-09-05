//! Substrate review probe (E): bare timer slop on this substrate. Throwaway.
use std::time::Duration;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

#[test]
fn e1_timer_slop_current_thread() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("rt");
    let mut bare = Vec::new();
    for _ in 0..5 {
        let t0 = std::time::Instant::now();
        rt.block_on(async { tokio::time::sleep(ms(20)).await });
        bare.push(t0.elapsed());
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let s = stop.clone();
    let busy = rt.spawn_blocking(move || {
        while !s.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(ms(5));
        }
    });
    let mut with_busy = Vec::new();
    for _ in 0..5 {
        let t0 = std::time::Instant::now();
        rt.block_on(async { tokio::time::sleep(ms(20)).await });
        with_busy.push(t0.elapsed());
    }
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    rt.block_on(async { let _ = busy.await; });
    let mt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_time()
        .build()
        .expect("rt");
    let mut multi = Vec::new();
    for _ in 0..5 {
        let t0 = std::time::Instant::now();
        mt.block_on(async { tokio::time::sleep(ms(20)).await });
        multi.push(t0.elapsed());
    }
    println!("E1 sleep(20ms) current_thread bare={bare:?}\n   with a busy blocking thread={with_busy:?}\n   multi_thread(2)={multi:?}");
}
