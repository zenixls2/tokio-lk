#![feature(test)]
extern crate test;
use std::time::Instant;
use test::Bencher;
use futures::future::join_all;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::cell::UnsafeCell;
use tokio_lk::*;

#[bench]
fn test_lock1000_parallel(_b: &mut Bencher) {
    let mut rt = Runtime::new().unwrap();
    let counter = UnsafeCell::new(0_u32);
    let map = Arc::new(RwLock::new(HashMap::new()));
    let now = Instant::now();
    let mut v = vec![];
    for _ in 0..1000 {
        let c = counter.get().clone() as u64;
        let task = Lock::fnew(2, map.clone())
            .and_then(|lock| lock)
            .and_then(move |guard| {
                unsafe {
                    *(c as *mut u32) += 1;
                }
                drop(guard);
                Ok(())
            });
        v.push(task);
    }
    rt.block_on(join_all(v)).unwrap();
    println!("test_lock1000_parallel: total elapsed time: {}ns", now.elapsed().as_nanos());
    assert_eq!(1000, counter.into_inner());
}
