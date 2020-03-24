#![feature(test)]
extern crate test;
use dashmap::DashMap;
use futures::future::join_all;
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::time::Instant;
use test::Bencher;
use tokio::runtime::Runtime;
use tokio_lk::*;

#[bench]
fn test_lock1000_parallel(_b: &mut Bencher) {
    let mut rt = Runtime::new().unwrap();
    let counter = UnsafeCell::new(0_u32);
    let map = Arc::new(DashMap::new());
    let mut v = vec![];
    for _ in 0..1000 {
        let task = async {
            let c = counter.get().clone() as u64;
            let guard = Lock::fnew(2, map.clone()).await.await;
            unsafe {
                *(c as *mut u32) += 1;
            }
            drop(guard);
        };
        v.push(task);
    }

    let now = Instant::now();
    rt.block_on(join_all(v));
    println!(
        "test_lock1000_parallel: total elapsed time: {}ns",
        now.elapsed().as_nanos()
    );
    assert_eq!(1000, counter.into_inner());
}
