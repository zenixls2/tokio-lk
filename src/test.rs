extern crate test;
use super::*;
use crossbeam::atomic::AtomicConsume;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::timer::Delay;

#[test]
fn test_drop() {
    let map = Arc::new(RwLock::new(HashMap::new()));
    let lock1 = Lock::new(1, map.clone());
    let lock2 = Lock::new(1, map.clone());
    assert_eq!(map.read().unwrap().get(&1).is_some(), true);
    match map.read().unwrap().get(&1) {
        Some(v) => {
            assert_eq!(v.0.load_consume(), 0);
            assert_eq!(v.1.load_consume(), 2);
        }
        None => unreachable!(),
    }
    drop(lock1);
    drop(lock2);
    assert_eq!(map.read().unwrap().len(), 0);
}

#[test]
fn test_future_drop() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(RwLock::new(HashMap::new()));
    let lock = Lock::new(1, map.clone());
    let c = Arc::new(AtomicUsize::new(0));
    let cc = c.clone();
    let task = lock.and_then(move |_| {
        cc.store(1, Relaxed);
        Ok(())
    });
    rt.block_on(task).unwrap();
    assert_eq!(c.load_consume(), 1);
    assert_eq!(map.read().unwrap().len(), 0);
}

#[test]
fn test_poll() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(RwLock::new(HashMap::new()));
    let mut lock = Lock::new(1, map.clone());
    let map2 = map.clone();
    let task = Ok(())
        .into_future()
        .map_err(|()| ())
        .and_then(move |_| {
            let guard = lock.poll().unwrap();
            let mut lock2 = Lock::new(1, map2.clone());
            assert!(lock2.poll().unwrap().is_not_ready());
            Ok(guard)
        })
        .map_err(|()| unreachable!());
    let _guard = rt.block_on(task).unwrap();
    let value = map.read().unwrap().get(&1).unwrap().clone();
    assert!(value.0.load_consume() > 0);
    assert_eq!(value.1.load_consume(), 1);
}

#[test]
fn test_future_multiple() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(RwLock::new(HashMap::new()));
    let map2 = map.clone();
    let now = Instant::now();
    let now2 = now.clone();
    let task1 = Ok(())
        .into_future()
        .and_then(move |_| {
            let lock = Lock::new(2, map2.clone());
            lock.and_then(move |guard| {
                Delay::new(now2 + Duration::from_millis(300))
                    .map_err(|_| ())
                    .map(move |_| guard)
            })
        })
        .and_then(|_| Ok(()));
    let task2 = Delay::new(now.clone() + Duration::from_millis(100))
        .map_err(|_| ())
        .and_then(move |_| {
            let lock = Lock::new(2, map);
            lock.and_then(move |_guard| {
                assert!(now.elapsed() >= Duration::from_millis(300));
                Ok(())
            })
        });
    rt.block_on(task1.join(task2)).unwrap();
}

#[test]
fn test_future_new_multiple() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(RwLock::new(HashMap::new()));
    let now = Instant::now();
    let task1 = Lock::fnew(1, map.clone())
        .and_then(|lock| lock)
        .and_then(|guard| {
            Delay::new(Instant::now() + Duration::from_millis(100))
                .map_err(|_| ())
                .map(move |_| guard)
        })
        .and_then(|_| Ok(()));
    let task2 = Lock::fnew(1, map.clone())
        .and_then(|lock| lock)
        .and_then(|guard| {
            Delay::new(Instant::now() + Duration::from_millis(100))
                .map_err(|_| ())
                .map(move |_| guard)
        })
        .and_then(|_| Ok(()));
    let task3 = Lock::fnew(2, map.clone())
        .and_then(|lock| lock)
        .and_then(|guard| {
            Delay::new(Instant::now() + Duration::from_millis(100))
                .map_err(|_| ())
                .map(move |_| guard)
        })
        .and_then(|_| Ok(()));
    rt.block_on(task1.join3(task2, task3)).unwrap();
    assert!(now.elapsed() >= Duration::from_millis(200));
    assert!(now.elapsed() <= Duration::from_millis(300));
}
