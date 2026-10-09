use akar_storage::version_info::VectorVersionInfo;
use criterion::{Criterion, criterion_group, criterion_main};
use std::collections::HashMap;

fn bench_vector_version_info_is_visible(c: &mut Criterion) {
    let vvi = VectorVersionInfo::new();
    let mut commit_history = HashMap::new();

    // Insert 10 transactions, each inserting 100 row indices
    for txn_id in 1..=10u64 {
        commit_history.insert(txn_id, txn_id * 10);
        for row in 0..100u32 {
            vvi.insert(txn_id, (txn_id as u32 - 1) * 100 + row);
        }
    }

    c.bench_function("VectorVersionInfo/is_visible", |b| {
        b.iter(|| {
            // Check visibility for row 550 at snapshot_ts 100
            vvi.is_visible(550, 100, &commit_history)
        })
    });
}

criterion_group!(benches, bench_vector_version_info_is_visible);
criterion_main!(benches);
