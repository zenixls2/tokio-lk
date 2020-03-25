#![feature(test)]
extern crate test;
use futures::future::join_all;
use std::cell::UnsafeCell;
use std::time::Instant;
use test::Bencher;
use tokio::runtime::Runtime;
use tokio_lk::*;

#[cfg(feature = "hashbrown")]
#[bench]
fn test_lock1000_hashbrown(_b: &mut Bencher) {
    let mut rt = Runtime::new().unwrap();
    let counter = UnsafeCell::new(0_u32);
    let map = KeyPool::<MapType>::new();
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
        "test_lock1000_hashbrown: total elapsed time: {}ns",
        now.elapsed().as_nanos()
    );
    assert_eq!(1000, counter.into_inner());
}

#[cfg(feature = "dashmap")]
#[bench]
fn test_lock1000_dashmap(_b: &mut Bencher) {
    let mut rt = Runtime::new().unwrap();
    let counter = UnsafeCell::new(0_u32);
    let map = KeyPool::<DashMapType>::new();
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
        "test_lock1000_dashmap: total elapsed time: {}ns",
        now.elapsed().as_nanos()
    );
    assert_eq!(1000, counter.into_inner());
}
