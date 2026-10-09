use akar_storage::free_space_manager::{FreeSpaceManager, PageRange};
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_total_free_pages(c: &mut Criterion) {
    let fsm = FreeSpaceManager::new();
    // Populate free lists across various levels
    for i in 0..1000u64 {
        fsm.add_free_pages(PageRange::new(i * 100, (i % 50) + 1));
    }

    c.bench_function("FreeSpaceManager/total_free_pages", |b| {
        b.iter(|| {
            fsm.total_free_pages()
        })
    });
}

criterion_group!(benches, bench_total_free_pages);
criterion_main!(benches);
