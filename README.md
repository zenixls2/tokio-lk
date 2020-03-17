[![Crates.io](https://img.shields.io/crates/v/tokio-lk.svg)](https://crates.io/crates/tokio-lk)
[![License](https://img.shields.io/crates/l/tokio-lk)](LICENSE-MIT)
[![Build Status](https://travis-ci.org/zenixls2/tokio-lk.svg?branch=master)](https://travis-ci.org/zenixls2/tokio-lk)

# tokio-lk version - 0.1.3

## Tokio-lk

** A lock-by-id future for tokio **

`Lock` future will return `Guard` once it gets the mutex.
to hold the lock in subsequent futures, move the `Guard` inside the future output.
to release the mutex, simply drops the `Guard` from your future chain.

Each `Lock` object is assigned an **unique** id.
The uniqueness is promised until USIZE_MAX of id gets generated.
Make sure old Locks are dropped before you generate new Locks above this amount.

### Changelog
- 0.1.3 - now depends on [dashmap](https://crates.io/crates/dashmap) to replace `RwLock<HashMap>`
- 0.1.2 - first stable version

### Example:
```rust
use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_lk::*;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use tokio::timer::Delay;

let mut rt = Runtime::new().unwrap();
let map = Arc::new(DashMap::new());
let now = Instant::now();
// this task will compete with task2 for lock at id 1
let task1 = Lock::fnew(1, map.clone())
    .and_then(|lock| Ok(lock))
    .and_then(|guard| {
        Delay::new(Instant::now() + Duration::from_millis(100))
            .map_err(|_| ())
            .map(move |_| guard)
    })
    .and_then(|_| Ok(()));
// this task will compete with task1 for lock at id 1
let task2 = Lock::fnew(1, map.clone())
    .and_then(|lock| Ok(lock))
    .and_then(|guard| {
        Delay::new(Instant::now() + Duration::from_millis(100))
            .map_err(|_| ())
            .map(move |_| guard)
    })
    .and_then(|_| Ok(()));
// no other task compete for lock at id 2
let task3 = Lock::fnew(2, map.clone())
    .and_then(|lock| Ok(lock))
    .and_then(|guard| {
        Delay::new(Instant::now() + Duration::from_millis(100))
            .map_err(|_| ())
            .map(move |_| guard)
    })
    .and_then(|_| Ok(()));
rt.block_on(task1.join3(task2, task3)).unwrap();
```

### Benchmark
to run the benchmark, execute the following command in the prompt:
```bash
cargo bench -- --nocapture
```
The `lock1000_parallel` benchmark is to run 1000 futures locked by a single lock to update the
counter.
The `lock1000_serial` benchmark is to run run similar operations in a single thread.
Currently our implementation is about 6 times slower than the single threaded version.

### License

Licensed under

* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licences/MIT)

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, shall be licensed as above,
without any additional terms or conditions.
