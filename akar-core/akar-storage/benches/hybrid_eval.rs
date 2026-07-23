use akar_storage::csr::CsrIndex;
use akar_storage::page::FileHandle;
use criterion::{Criterion, criterion_group, criterion_main};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

fn bench_csr_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("LadybugDB Hybrid CSR");

    group.bench_function("insert_edge", |b| {
        let file_handle = Arc::new(RwLock::new(FileHandle::new(PathBuf::from("mock.db"), 4096)));
        let mut csr = CsrIndex::new(file_handle);

        b.iter(|| {
            let src = std::hint::black_box(1);
            let dst = std::hint::black_box(2);
            let _ = csr.add_edge(src, dst);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_csr_insert);
criterion_main!(benches);
