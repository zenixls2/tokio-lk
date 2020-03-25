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
//! - 0.2.1 - add features for using either hashbrown or dashmap. add `KeyPool` for hashmap abstraction.
//! - 0.2.0 - bump to futures 0.3 and tokio 0.2
//! - 0.1.3 - now depends on [dashmap](https://crates.io/crates/dashmap) to replace `RwLock<HashMap>`
//! - 0.1.2 - first stable version
//!
//! ## Example:
//! ```rust,no_run
//! use std::time::{Duration, Instant};
//! use tokio_lk::*;
//! use futures::prelude::*;
//! use tokio::runtime::Runtime;
//! use tokio::time::delay_for;
//!
//! let mut rt = Runtime::new().unwrap();
//! let map = KeyPool::<MapType>::new();
//! let now = Instant::now();
//! // this task will compete with task2 for lock at id 1
//! let task1 = async {
//!     let _guard = Lock::fnew(1, map.clone()).await.await;
//!     delay_for(Duration::from_millis(100)).await;
//! };
//! // this task will compete with task1 for lock at id 1
//! let task2 = async {
//!     let _guard = Lock::fnew(1, map.clone()).await.await;
//!     delay_for(Duration::from_millis(100)).await;
//! };
//! // no other task compete for lock at id 2
//! let task3 = async {
//!     let _guard = Lock::fnew(2, map.clone()).await.await;
//!     delay_for(Duration::from_millis(100)).await;
//! };
//! rt.block_on(async { tokio::join!(task1, task2, task3) });
//! ```
//!
//! ## Features
//! - hashbrown
//!     * provides `MapType` as a type alias of `hashbrown::HashMap` for KeyPool initialization
//! - dashmap
//!     * provides `DashMapType` as a type alias of `dashmap::DashMap` for KeyPool initialization
//! - default: hashbrown
//! - all: both `hashbrown` and `dashmap`
//!
//! ## Benchmark
//! to run the benchmark, execute the following command in the prompt:
//! ```bash
//! cargo bench -- --nocapture
//! ```
//! The `lock1000_parallel` benchmark is to run 1000 futures locked by a single lock to update the
//! counter.
//! The `lock1000_serial` benchmark is to run run similar operations in a single thread.
//! Currently our implementation is about 4~5 times slower than the single threaded version.
#![feature(test, specialization)]

#[cfg(feature = "dashmap")]
use dashmap::DashMap;
use futures::prelude::*;
#[cfg(feature = "hashbrown")]
use hashbrown::HashMap;
use lazy_static::lazy_static;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering::*};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
mod atomic_serial_waker;
use atomic_serial_waker::AtomicSerialWaker;
use std::fmt;
#[cfg(test)]
mod test;

/// the map type used to store lock keys
#[cfg(feature = "hashbrown")]
pub type MapType = Arc<Mutex<HashMap<usize, Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>>>>;
#[cfg(feature = "dashmap")]
pub type DashMapType = Arc<DashMap<usize, Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>>>;

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

pub struct KeyPool<T> {
    table: T,
}

pub trait NewKeyPool<T> {
    fn new() -> Self;
}

#[cfg(feature = "hashbrown")]
impl NewKeyPool<MapType> for KeyPool<MapType> {
    #[inline]
    fn new() -> Self {
        Self {
            table: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[cfg(feature = "dashmap")]
impl NewKeyPool<DashMapType> for KeyPool<DashMapType> {
    #[inline]
    fn new() -> Self {
        Self {
            table: Arc::new(DashMap::new()),
        }
    }
}

impl<T: Clone> Clone for KeyPool<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            table: self.table.clone(),
        }
    }
}

impl<T> Deref for KeyPool<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.table
    }
}

impl<T: fmt::Debug> fmt::Debug for KeyPool<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.table.fmt(f)
    }
}

impl<T> DerefMut for KeyPool<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.table
    }
}

/// ### Lock future struct
/// The lock future refers to shared map to support lock-by-id functionality
#[derive(Debug)]
pub struct Lock<T>
where
    Lock<T>: Destruct<T>,
{
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
    map: KeyPool<T>,
    id: usize,
    has_guard: bool,
}

pub struct AsyncInsert<T> {
    pub(crate) target: usize,
    pub(crate) map: KeyPool<T>,
}

#[cfg(feature = "hashbrown")]
impl Future for AsyncInsert<MapType> {
    type Output = Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>;
    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let pinned = Pin::get_mut(self);
        match pinned.map.try_lock() {
            Ok(mut wmap) => {
                let value = wmap
                    .entry(pinned.target)
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
                Poll::Ready(value)
            }
            Err(_) => {
                TASK.register(cx.waker());
                Poll::Pending
            }
        }
    }
}

#[cfg(feature = "dashmap")]
impl Future for AsyncInsert<DashMapType> {
    type Output = Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>;
    #[inline]
    fn poll(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Self::Output> {
        let pinned = Pin::get_mut(self);
        let value = pinned
            .map
            .entry(pinned.target)
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
        Poll::Ready(value)
    }
}

impl<T: Clone> Lock<T>
where
    Lock<T>: Destruct<T>,
    AsyncInsert<T>: Future<Output = Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>>,
{
    /// Create a Lock instance on the future's result
    #[inline]
    pub async fn fnew(target: usize, map: KeyPool<T>) -> Self {
        let map2 = map.clone();
        let value = AsyncInsert { target, map }.await;
        TASK.wake();
        Self {
            target,
            value,
            map: map2,
            id: get_id(),
            has_guard: false,
        }
    }
}

pub trait New<T: Clone>: Destruct<T> {
    fn new(target: usize, map: KeyPool<T>) -> Self;
}

#[cfg(feature = "hashbrown")]
impl New<MapType> for Lock<MapType> {
    /// Create a Lock instance.
    /// This operation might block threads from parking for a while
    /// Don't use this function inside tokio context
    /// please refer to `fnew` function to provide asynchrons `Lock` instance generation
    #[inline]
    fn new(target: usize, map: KeyPool<MapType>) -> Self {
        let id = get_id();
        let value = map
            .lock()
            .unwrap()
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
}

#[cfg(feature = "dashmap")]
impl New<DashMapType> for Lock<DashMapType> {
    /// Create a Lock instance.
    /// This operation might block threads from parking for a while
    /// Don't use this function inside tokio context
    /// please refer to `fnew` function to provide asynchrons `Lock` instance generation
    #[inline]
    fn new(target: usize, map: KeyPool<DashMapType>) -> Self {
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
}

pub trait Destruct<T> {
    fn destruct(&mut self);
}

default impl<T> Destruct<T> for Lock<T> {
    #[inline]
    fn destruct(&mut self) {}
}

#[cfg(feature = "hashbrown")]
impl Destruct<MapType> for Lock<MapType> {
    #[inline]
    fn destruct(&mut self) {
        if !self.has_guard && self.value.1.fetch_sub(1, Relaxed) == 1 {
            self.map.lock().unwrap().remove(&self.target);
        }
    }
}

#[cfg(feature = "dashmap")]
impl Destruct<DashMapType> for Lock<DashMapType> {
    #[inline]
    fn destruct(&mut self) {
        if !self.has_guard && self.value.1.fetch_sub(1, Relaxed) == 1 {
            self.map.remove(&self.target);
        }
    }
}

default impl<T> Destruct<T> for Guard<T> {
    #[inline]
    fn destruct(&mut self) {}
}

#[cfg(feature = "hashbrown")]
impl Destruct<MapType> for Guard<MapType> {
    #[inline]
    fn destruct(&mut self) {
        self.value.0.swap(0, Relaxed);
        self.value.2.wake();
        if self.value.1.fetch_sub(1, AcqRel) == 1 {
            self.map.lock().unwrap().remove(&self.target);
        }
    }
}

#[cfg(feature = "dashmap")]
impl Destruct<DashMapType> for Guard<DashMapType> {
    #[inline]
    fn destruct(&mut self) {
        self.value.0.swap(0, Relaxed);
        self.value.2.wake();
        if self.value.1.fetch_sub(1, AcqRel) == 1 {
            self.map.remove(&self.target);
        }
    }
}

impl<T: Clone> Clone for Lock<T>
where
    Lock<T>: Destruct<T>,
{
    #[inline]
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

impl<T: Clone + Unpin> Future for Lock<T>
where
    Lock<T>: Destruct<T>,
    Guard<T>: Destruct<T>,
{
    type Output = Guard<T>;
    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let pinned = Pin::get_mut(self);
        pinned.value.2.register(cx.waker());
        if let Err(prev) = pinned
            .value
            .0
            .compare_exchange_weak(0, pinned.id, SeqCst, SeqCst)
        {
            if prev == pinned.id {
                pinned.value.2.wake();
                return Poll::Ready(Guard::new(
                    pinned.target,
                    pinned.value.clone(),
                    pinned.map.clone(),
                ));
            }
            return Poll::Pending;
        }
        pinned.has_guard = true;
        pinned.value.2.wake();
        Poll::Ready(Guard::new(
            pinned.target,
            pinned.value.clone(),
            pinned.map.clone(),
        ))
    }
}

/// the `Guard` functions like `MutexGuard` in the standard library.
/// hold it, and then you hold the mutex.
/// drop it, and you release the lock.
#[derive(Debug)]
pub struct Guard<T>
where
    Guard<T>: Destruct<T>,
{
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
    map: KeyPool<T>,
}

impl<T> Guard<T>
where
    Guard<T>: Destruct<T>,
{
    #[inline]
    pub fn new(
        target: usize,
        value: Arc<(AtomicUsize, AtomicUsize, AtomicSerialWaker)>,
        map: KeyPool<T>,
    ) -> Self {
        Self { target, value, map }
    }
}

unsafe impl<T> Send for Guard<T> where Guard<T>: Destruct<T> {}
unsafe impl<T> Sync for Guard<T> where Guard<T>: Destruct<T> {}

impl<T> Drop for Guard<T>
where
    Guard<T>: Destruct<T>,
{
    #[inline]
    fn drop(&mut self) {
        self.destruct();
    }
}

impl<T> Drop for Lock<T>
where
    Lock<T>: Destruct<T>,
{
    #[inline]
    fn drop(&mut self) {
        self.destruct();
    }
}
