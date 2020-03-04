#![feature(test)]

use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::*};
use std::sync::{Arc, RwLock};
use tokio::prelude::{task::AtomicTask, *};
#[cfg(test)]
mod test;

pub type MapType = Arc<RwLock<HashMap<usize, Arc<(AtomicUsize, AtomicUsize, AtomicTask)>>>>;

lazy_static! {
    static ref ID: AtomicUsize = AtomicUsize::new(1);
    static ref TASK: AtomicTask = AtomicTask::new();
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

#[derive(Debug)]
pub struct Lock {
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicTask)>,
    map: MapType,
    id: usize,
    has_guard: bool,
}

struct AsyncInsert {
    pub(crate) target: usize,
    pub(crate) map: MapType,
}

impl Future for AsyncInsert {
    type Item = Arc<(AtomicUsize, AtomicUsize, AtomicTask)>;
    type Error = ();
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.map.try_write() {
            Ok(mut wmap) => {
                let value = wmap
                    .entry(self.target)
                    .and_modify(|e| {
                        e.1.fetch_add(1, Relaxed);
                    })
                    .or_insert_with(|| {
                        Arc::new((AtomicUsize::new(0), AtomicUsize::new(0), AtomicTask::new()))
                    })
                    .clone();
                Ok(Async::Ready(value))
            }
            Err(_) => {
                TASK.register();
                Ok(Async::NotReady)
            }
        }
    }
}

impl Lock {
    pub fn new(target: usize, map: MapType) -> Self {
        let id = get_id();
        let mut wmap = map.write().unwrap();
        let value = wmap
            .entry(target)
            .and_modify(|e| {
                e.1.fetch_add(1, Relaxed);
            })
            .or_insert_with(|| {
                Arc::new((AtomicUsize::new(0), AtomicUsize::new(1), AtomicTask::new()))
            })
            .clone();
        drop(wmap);
        TASK.notify();
        Self {
            target,
            value,
            map,
            id,
            has_guard: false,
        }
    }
    pub fn fnew(target: usize, map: MapType) -> impl Future<Item = Self, Error = ()> {
        let map2 = map.clone();
        let task = AsyncInsert { target, map };
        task.and_then(move |value| {
            TASK.notify();
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
        if let Err(prev) = self
            .value
            .0
            .compare_exchange_weak(0, self.id, SeqCst, SeqCst)
        {
            if prev == self.id {
                self.has_guard = true;
                return Ok(Async::Ready(Guard::new(
                    self.target,
                    self.value.clone(),
                    self.map.clone(),
                )));
            }
            self.value.2.register();
            return Ok(Async::NotReady);
        }
        self.has_guard = true;
        Ok(Async::Ready(Guard::new(
            self.target,
            self.value.clone(),
            self.map.clone(),
        )))
    }
}
#[derive(Debug)]
pub struct Guard {
    target: usize,
    value: Arc<(AtomicUsize, AtomicUsize, AtomicTask)>,
    map: MapType,
}

impl Guard {
    pub fn new(
        target: usize,
        value: Arc<(AtomicUsize, AtomicUsize, AtomicTask)>,
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
        self.value.2.notify();
        let mut map = self.map.write().unwrap();
        if self.value.1.fetch_sub(1, Relaxed) == 1 {
            map.remove(&self.target);
        }
        drop(map);
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if !self.has_guard {
            let mut map = self.map.write().unwrap();
            if self.value.1.fetch_sub(1, Relaxed) == 1 {
                map.remove(&self.target);
            }
            drop(map);
        }
    }
}
