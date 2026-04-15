//! Micro-benchmarks for the PersistentStore wrapper around LocalDatabase.
//!
//! Every insert/update persists the entire JSON store to disk via a temp-write
//! plus rename. These benches measure that cost at varying store sizes and under
//! mixed read/write load.
//!
//! Note: the store's `insert` flushes to disk synchronously, so absolute numbers
//! depend on the filesystem. Use these benches to detect regressions, not to
//! quote absolute latencies.

use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tempfile::TempDir;

use sandbox_runtime::store::PersistentStore;

fn make_store(size: usize) -> (PersistentStore<String>, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("store.json");
    let store: PersistentStore<String> = PersistentStore::open(path).expect("open");
    for i in 0..size {
        store
            .insert(format!("k{i}"), format!("v{i}"))
            .expect("seed insert");
    }
    (store, dir)
}

fn bench_insert_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("store/insert");
    group.throughput(Throughput::Elements(1));

    for &n in &[100usize, 1_000] {
        let (store, _dir) = make_store(n);
        let mut i = 0u64;
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let key = format!("bench-{i}");
                i = i.wrapping_add(1);
                store
                    .insert(black_box(key), black_box("payload".to_string()))
                    .expect("insert");
            })
        });
    }
    group.finish();
}

fn bench_get_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("store/get");
    group.throughput(Throughput::Elements(1));

    for &n in &[100usize, 1_000, 10_000] {
        let (store, _dir) = make_store(n);
        let mut i = 0usize;
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let key = format!("k{}", i % n);
                i = i.wrapping_add(1);
                let _ = black_box(store.get(black_box(&key)).expect("get"));
            })
        });
    }
    group.finish();
}

fn bench_update_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("store/update");
    group.throughput(Throughput::Elements(1));

    for &n in &[100usize, 1_000] {
        let (store, _dir) = make_store(n);
        let mut i = 0usize;
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let key = format!("k{}", i % n);
                i = i.wrapping_add(1);
                store
                    .update(black_box(&key), |v| {
                        v.push('x');
                    })
                    .expect("update");
            })
        });
    }
    group.finish();
}

fn bench_find_scaling(c: &mut Criterion) {
    // `find` is O(n) linear scan — worst case for large stores.
    let mut group = c.benchmark_group("store/find_linear_scan");
    group.throughput(Throughput::Elements(1));

    for &n in &[100usize, 1_000, 10_000] {
        let (store, _dir) = make_store(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                // Predicate that matches nothing — forces full scan.
                let _ = black_box(store.find(|v| v == "zzz-none").expect("find"));
            })
        });
    }
    group.finish();
}

fn bench_values_scaling(c: &mut Criterion) {
    // `values` clones every entry — dominates when the reaper/GC runs.
    let mut group = c.benchmark_group("store/values_clone_all");

    for &n in &[100usize, 1_000, 10_000] {
        let (store, _dir) = make_store(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let v = store.values().expect("values");
                black_box(v);
            })
        });
    }
    group.finish();
}

fn bench_concurrent_readers_single_writer(c: &mut Criterion) {
    // Concurrent read pressure: 4 reader threads doing get() while one writer
    // does insert(). Measures how the RwLock behaves under mixed load.
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    let (store, _dir) = make_store(1_000);
    let store = Arc::new(store);
    let stop = Arc::new(AtomicBool::new(false));

    // Spawn readers
    let mut readers = Vec::new();
    for _ in 0..4 {
        let s = Arc::clone(&store);
        let stop_flag = Arc::clone(&stop);
        readers.push(thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                let _ = s.get("k500").ok();
            }
        }));
    }

    // The writer is what we're measuring.
    let mut i = 0u64;
    c.bench_function("store/insert_under_read_load", |b| {
        b.iter(|| {
            let key = format!("w-{i}");
            i = i.wrapping_add(1);
            store.insert(key, "payload".to_string()).expect("insert");
        })
    });

    stop.store(true, Ordering::Relaxed);
    for t in readers {
        t.join().expect("reader join");
    }
}

criterion_group!(
    store_benches,
    bench_insert_scaling,
    bench_get_scaling,
    bench_update_scaling,
    bench_find_scaling,
    bench_values_scaling,
    bench_concurrent_readers_single_writer,
);
criterion_main!(store_benches);
