use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kuzu_storage::csr::CsrIndex;
use kuzu_storage::page::FileHandle;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

fn bench_csr_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("LadybugDB Hybrid CSR");

    group.bench_function("insert_edge", |b| {
        let file_handle = Arc::new(RwLock::new(FileHandle::new(PathBuf::from("mock.db"), 4096)));
        let mut csr = CsrIndex::new(file_handle);

        b.iter(|| {
            let src = black_box(1);
            let dst = black_box(2);
            let _ = csr.add_edge(src, dst);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_csr_insert);
criterion_main!(benches);
