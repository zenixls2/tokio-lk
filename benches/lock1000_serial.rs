#![feature(test)]
extern crate test;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use test::Bencher;

#[bench]
fn test_lock1000_serial(_b: &mut Bencher) {
    let map = Arc::new(RwLock::new(HashMap::new()));
    let now = Instant::now();
    let mut wmap = map.write().unwrap();
    wmap.entry(1).or_insert_with(|| Arc::new(Mutex::new(0_u32)));
    drop(wmap);
    for _ in 0..1000 {
        let value = map.read().unwrap().get(&1).unwrap().clone();
        *value.lock().unwrap() += 1;
    }
    println!(
        "test_lock1000_serial: total elapsed time: {}ns",
        now.elapsed().as_nanos()
    );
    assert_eq!(
        1000,
        *(map.read().unwrap().get(&1).unwrap().lock().unwrap())
    );
}
