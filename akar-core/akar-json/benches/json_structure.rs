use std::hint::black_box;
use std::time::Duration;

use akar_json::json_structure_of;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

fn generate_array_json(num_elements: usize) -> String {
    let mut json = String::with_capacity(num_elements * 20 + 2);
    json.push('[');
    for i in 0..num_elements {
        if i > 0 {
            json.push(',');
        }
        use std::fmt::Write;
        write!(&mut json, "{{\"key_{}\": {}}}", i, i).unwrap();
    }
    json.push(']');
    json
}

fn bench_json_structure(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_structure");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(3));

    for &size in &[100usize, 1_000, 5_000] {
        let json_str = generate_array_json(size);
        group.bench_with_input(BenchmarkId::new("array_objects", size), &json_str, |b, json| {
            b.iter(|| json_structure_of(black_box(json)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_json_structure);
criterion_main!(benches);
