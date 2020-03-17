//! # Tokio-lk
//!
//! ** A lock-by-id future for tokio **
//!
//! `Lock` future will return `Guard` once it gets the mutex.
//! to hold the lock in subsequent futures, move the `Guard` inside the future output.
//! to release the mutex, simply drops the `Guard` from your future chain.
//!
//! Each `Lock` object is assigned an **unique** id.
//! The uniqueness is promised until USIZE_MAX of id gets generated.
//! Make sure old Locks are dropped before you generate new Locks above this amount.
//!
//! ## Changelog
//! - 0.1.3 - now depends on [dashmap](https://crates.io/crates/dashmap) to replace `RwLock<HashMap>`
//! - 0.1.2 - first stable version
//!
//! ## Example:
//! ```rust,no_run
//! use dashmap::DashMap;
//! use std::sync::Arc;
//! use std::time::{Duration, Instant};
//! use tokio_lk::*;
//! use tokio::prelude::*;
//! use tokio::runtime::Runtime;
//! use tokio::timer::Delay;
//!
//! let mut rt = Runtime::new().unwrap();
//! let map = Arc::new(DashMap::new());
//! let now = Instant::now();
//! // this task will compete with task2 for lock at id 1
//! let task1 = Lock::fnew(1, map.clone())
//!     .and_then(|lock| Ok(lock))
//!     .and_then(|guard| {
//!         Delay::new(Instant::now() + Duration::from_millis(100))
//!             .map_err(|_| ())
//!             .map(move |_| guard)
//!     })
//!     .and_then(|_| Ok(()));
//! // this task will compete with task1 for lock at id 1
//! let task2 = Lock::fnew(1, map.clone())
//!     .and_then(|lock| Ok(lock))
//!     .and_then(|guard| {
//!         Delay::new(Instant::now() + Duration::from_millis(100))
//!             .map_err(|_| ())
//!             .map(move |_| guard)
//!     })
//!     .and_then(|_| Ok(()));
//! // no other task compete for lock at id 2
//! let task3 = Lock::fnew(2, map.clone())
//!     .and_then(|lock| Ok(lock))
//!     .and_then(|guard| {
//!         Delay::new(Instant::now() + Duration::from_millis(100))
//!             .map_err(|_| ())
//!             .map(move |_| guard)
//!     })
//!     .and_then(|_| Ok(()));
//! rt.block_on(task1.join3(task2, task3)).unwrap();
//! ```
//!
//! ## Benchmark
//! to run the benchmark, execute the following command in the prompt:
//! ```bash
//! cargo bench -- --nocapture
//! ```
//! The `lock1000_parallel` benchmark is to run 1000 futures locked by a single lock to update the
//! counter.
//! The `lock1000_serial` benchmark is to run run similar operations in a single thread.
//! Currently our implementation is about 6 times slower than the single threaded version.
#![feature(test)]

use dashmap::DashMap;
use lazy_static::lazy_static;
use std::sync::atomic::{AtomicUsize, Ordering::*};
use std::sync::Arc;
use tokio::prelude::*;
mod atomic_serial_waker;
use atomic_serial_waker::AtomicSerialWaker;
#[cfg(test)]
mod test;

/// the map type used to store lock keys
pub type MapType = Arc<DashMap<usize, Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>>>;

lazy_static! {
    static ref ID: AtomicUsize = AtomicUsize::new(1);
    static ref TASK: AtomicSerialWaker = AtomicSerialWaker::new();
}

#[inline]
fn get_id() -> usize {
    let result = ID.fetch_add(1, Relaxed);
    if result == 0 {
        ID.fetch_add(1, Relaxed)
    } else {
        result
    }
}

/// ### Lock future struct
/// The lock future refers to shared map to support lock-by-id functionality
#[derive(Debug)]
pub struct Lock {
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
    map: MapType,
    id: usize,
    has_guard: bool,
}

struct AsyncInsert {
    pub(crate) target: usize,
    pub(crate) map: MapType,
}

impl Future for AsyncInsert {
    type Item = Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>;
    type Error = ();
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let value = self
            .map
            .entry(self.target)
            .and_modify(|e| {
                e.1.fetch_add(1, Relaxed);
            })
            .or_insert_with(|| {
                Arc::new((
                    AtomicUsize::new(0),
                    AtomicUsize::new(0),
                    AtomicSerialWaker::new(),
                ))
            })
            .clone();
        Ok(Async::Ready(value))
    }
}

impl Lock {
    /// Create a Lock instance.
    /// This operation might block threads from parking for a while
    /// Don't use this function inside tokio context
    /// please refer to `fnew` function to provide asynchrons `Lock` instance generation
    pub fn new(target: usize, map: MapType) -> Self {
        let id = get_id();
        let value = map
            .entry(target)
            .and_modify(|e| {
                e.1.fetch_add(1, Relaxed);
            })
            .or_insert_with(|| {
                Arc::new((
                    AtomicUsize::new(0),
                    AtomicUsize::new(1),
                    AtomicSerialWaker::new(),
                ))
            })
            .clone();
        TASK.wake();
        Self {
            target,
            value,
            map,
            id,
            has_guard: false,
        }
    }
    /// Create a Lock instance on the future's result
    pub fn fnew(target: usize, map: MapType) -> impl Future<Item = Self, Error = ()> {
        let map2 = map.clone();
        let task = AsyncInsert { target, map };
        task.and_then(move |value| {
            TASK.wake();
            Ok(Self {
                target,
                value,
                map: map2,
                id: get_id(),
                has_guard: false,
            })
        })
    }
}

impl Clone for Lock {
    fn clone(&self) -> Self {
        self.value.1.fetch_add(1, Relaxed);
        Self {
            target: self.target,
            value: self.value.clone(),
            map: self.map.clone(),
            id: get_id(),
            has_guard: false,
        }
    }
}

impl Future for Lock {
    type Item = Guard;
    type Error = ();
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.value.2.register();
        if let Err(prev) = self
            .value
            .0
            .compare_exchange_weak(0, self.id, SeqCst, SeqCst)
        {
            if prev == self.id {
                self.value.2.wake();
                return Ok(Async::Ready(Guard::new(
                    self.target,
                    self.value.clone(),
                    self.map.clone(),
                )));
            }
            return Ok(Async::NotReady);
        }
        self.has_guard = true;
        self.value.2.wake();
        Ok(Async::Ready(Guard::new(
            self.target,
            self.value.clone(),
            self.map.clone(),
        )))
    }
}

/// the `Guard` functions like `MutexGuard` in the standard library.
/// hold it, and then you hold the mutex.
/// drop it, and you release the lock.
#[derive(Debug)]
pub struct Guard {
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
    map: MapType,
}

impl Guard {
    pub fn new(
        target: usize,
        value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
        map: MapType,
    ) -> Self {
        Self { target, value, map }
    }
}

unsafe impl Send for Guard {}
unsafe impl Sync for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        self.value.0.swap(0, Relaxed);
        self.value.2.wake();
        if self.value.1.fetch_sub(1, AcqRel) == 1 {
            self.map.remove(&self.target);
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if !self.has_guard && self.value.1.fetch_sub(1, Relaxed) == 1 {
            self.map.remove(&self.target);
        }
    }
}
