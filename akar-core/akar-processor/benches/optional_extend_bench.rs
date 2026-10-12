use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use hashbrown::HashSet;
use std::collections::HashMap;

fn probe_incident_edges_old(
    fwd_adj: &HashMap<u64, Vec<(u64, usize)>>,
    rev_adj: &HashMap<u64, Vec<(u64, usize)>>,
    src: u64,
) -> Vec<usize> {
    let mut idxs: Vec<usize> = Vec::new();
    for list in [fwd_adj.get(&src), rev_adj.get(&src)].into_iter().flatten() {
        for (_, i) in list {
            if !idxs.contains(i) {
                idxs.push(*i);
            }
        }
    }
    idxs
}

fn probe_incident_edges_new(
    fwd_adj: &HashMap<u64, Vec<(u64, usize)>>,
    rev_adj: &HashMap<u64, Vec<(u64, usize)>>,
    src: u64,
) -> Vec<usize> {
    let fwd = fwd_adj.get(&src);
    let rev = rev_adj.get(&src);

    match (fwd, rev) {
        (None, None) => Vec::new(),
        (Some(fwd_list), None) => fwd_list.iter().map(|(_, i)| *i).collect(),
        (None, Some(rev_list)) => rev_list.iter().map(|(_, i)| *i).collect(),
        (Some(fwd_list), Some(rev_list)) => {
            let total = fwd_list.len() + rev_list.len();
            let mut idxs = Vec::with_capacity(total);
            if total <= 16 {
                for (_, i) in fwd_list.iter().chain(rev_list.iter()) {
                    if !idxs.contains(i) {
                        idxs.push(*i);
                    }
                }
            } else {
                let mut seen = HashSet::with_capacity(total);
                for (_, i) in fwd_list.iter().chain(rev_list.iter()) {
                    if seen.insert(*i) {
                        idxs.push(*i);
                    }
                }
            }
            idxs
        }
    }
}

fn bench_optional_extend(c: &mut Criterion) {
    let mut group = c.benchmark_group("probe_incident_edges");

    for size in [2, 10, 100, 1000, 10000] {
        let mut fwd_adj: HashMap<u64, Vec<(u64, usize)>> = HashMap::new();
        let mut rev_adj: HashMap<u64, Vec<(u64, usize)>> = HashMap::new();

        let src = 1u64;
        let fwd_list: Vec<(u64, usize)> = (0..size).map(|i| (2u64, i)).collect();
        let rev_list: Vec<(u64, usize)> = (size / 2..size + size / 2).map(|i| (3u64, i)).collect();

        fwd_adj.insert(src, fwd_list);
        rev_adj.insert(src, rev_list);

        group.bench_with_input(BenchmarkId::new("old_O_N2", size), &size, |b, _| {
            b.iter(|| probe_incident_edges_old(&fwd_adj, &rev_adj, src))
        });

        group.bench_with_input(BenchmarkId::new("new_O_N", size), &size, |b, _| {
            b.iter(|| probe_incident_edges_new(&fwd_adj, &rev_adj, src))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_optional_extend);
criterion_main!(benches);
