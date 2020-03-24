extern crate test;
use super::*;
use crossbeam::atomic::AtomicConsume;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time::delay_for;
use std::pin::Pin;

struct TestPoll(Lock);
impl Future for TestPoll {
    type Output = Poll<Guard>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let pinned = Pin::get_mut(self);
        Poll::Ready(Pin::new(&mut pinned.0).poll(cx))
    }
}

#[test]
fn test_drop() {
    let map = Arc::new(DashMap::new());
    let lock1 = Lock::new(1, map.clone());
    let lock2 = Lock::new(1, map.clone());
    assert_eq!(map.get(&1).is_some(), true);
    match map.get(&1) {
        Some(v) => {
            assert_eq!(v.0.load_consume(), 0);
            assert_eq!(v.1.load_consume(), 2);
        }
        None => unreachable!(),
    }
    drop(lock1);
    drop(lock2);
    assert_eq!(map.len(), 0);
}

#[test]
fn test_future_drop() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(DashMap::new());
    let lock = Lock::new(1, map.clone());
    let c = Arc::new(AtomicUsize::new(0));
    let cc = c.clone();
    let task = async move {
        let _guard = lock.await;
        cc.store(1, Relaxed);
    };
    rt.block_on(task);
    assert_eq!(c.load_consume(), 1);
    assert_eq!(map.len(), 0);
}

#[test]
fn test_poll() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(DashMap::new());
    let lock = Lock::new(1, map.clone());
    let map2 = map.clone();
    let task = async move {
        let guard = lock.await;
        let lock2 = TestPoll(Lock::new(1, map2.clone()));
        assert!(lock2.await.is_pending());
        guard
    };
    let _guard = rt.block_on(task);
    let value = map.get(&1).unwrap().clone();
    assert!(value.0.load_consume() > 0);
    assert_eq!(value.1.load_consume(), 1);
}

#[test]
fn test_future_multiple() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(DashMap::new());
    let map2 = map.clone();
    let now = Instant::now();
    let task1 = async {
        let lock = Lock::new(2, map2.clone());
        let _guard = lock.await;
        delay_for(Duration::from_millis(300)).await;
    };
    let task2 = async move {
        delay_for(Duration::from_millis(100)).await;
        let lock = Lock::new(2, map);
        let _guard = lock.await;
        assert!(now.elapsed() >= Duration::from_millis(300));
    };
    rt.block_on(async {
        tokio::join!(task1, task2)
    });
}

#[test]
fn test_future_new_multiple() {
    let mut rt = Runtime::new().unwrap();
    let map = Arc::new(DashMap::new());
    let now = Instant::now();
    let task1 = async {
        let _guard = Lock::fnew(1, map.clone()).await.await;
        delay_for(Duration::from_millis(100)).await;
    };
    let task2 = async {
        let _guard = Lock::fnew(1, map.clone()).await.await;
        delay_for(Duration::from_millis(100)).await;
    };
    let task3 = async {
        let _guard = Lock::fnew(2, map.clone()).await.await;
        delay_for(Duration::from_millis(100)).await;
    };
    rt.block_on(async {
        tokio::join!(task1, task2, task3)
    });
    assert!(now.elapsed() >= Duration::from_millis(200));
    assert!(now.elapsed() <= Duration::from_millis(300));
}
