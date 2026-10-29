use std::collections::HashSet;
use akar_vector::hnsw::{DistanceMetric, HnswIndex};

#[test]
fn test_algorithm4_diversity_heuristic() {
    let mut index = HnswIndex::new(DistanceMetric::Cosine);

    // Node 0: origin direction
    index.insert(vec![1.0, 0.0], 0);
    // Node 1, 2: closely clustered to Node 0
    index.insert(vec![0.99, 0.01], 1);
    index.insert(vec![0.98, 0.02], 2);
    // Node 3: orthogonal direction
    index.insert(vec![0.0, 1.0], 3);

    // Search query closer to [1.0, 0.0]
    let hits = index.search(&[0.95, 0.05], 3);
    assert!(!hits.is_empty(), "Should return hits");
    // Verify results are sorted by distance ascending
    for i in 1..hits.len() {
        assert!(hits[i - 1].0 <= hits[i].0, "Results must be sorted by distance");
    }
}

#[test]
fn test_tombstone_filtering_and_removal() {
    let mut index = HnswIndex::new(DistanceMetric::Euclidean);

    // Insert 20 vectors
    for i in 0..20 {
        index.insert(vec![i as f32, (i * 2) as f32], i);
    }
    assert_eq!(index.len(), 20);

    // Nearest to [0.0, 0.0] should be node 0
    let hits_before = index.search(&[0.0, 0.0], 5);
    assert_eq!(hits_before[0].1, 0, "Nearest should be node 0");

    // Tombstone node 0
    index.remove(0);
    assert!(index.is_deleted(0), "Node 0 must be marked deleted");
    assert_eq!(index.len(), 19, "Length should decrease after tombstone");

    // Search again: node 0 must NOT appear in search results
    let hits_after = index.search(&[0.0, 0.0], 5);
    for (_, id) in &hits_after {
        assert_ne!(*id, 0, "Tombstoned node 0 must not appear in search hits");
    }
    assert_eq!(hits_after[0].1, 1, "Nearest should now be node 1");

    // Re-insert node 0 with new vector
    index.insert(vec![0.1, 0.1], 0);
    assert!(!index.is_deleted(0), "Node 0 must not be deleted after re-insert");
    assert_eq!(index.len(), 20, "Length should be back to 20");
    let hits_reinserted = index.search(&[0.0, 0.0], 5);
    assert_eq!(hits_reinserted[0].1, 0, "Re-inserted node 0 should be found");
}

#[test]
fn test_hnsw_recall_against_brute_force() {
    let mut index = HnswIndex::new(DistanceMetric::Cosine);
    let num_vectors = 300;
    let dims = 16;
    let mut rng: u64 = 424242;

    let mut random_vec = |rng: &mut u64| -> Vec<f32> {
        let mut v = Vec::with_capacity(dims);
        for _ in 0..dims {
            *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            v.push(((*rng >> 33) as f32 / (1u32 << 31) as f32) - 0.5);
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    };

    let mut all_vectors = Vec::with_capacity(num_vectors);
    for id in 0..num_vectors {
        let v = random_vec(&mut rng);
        all_vectors.push((id, v.clone()));
        index.insert(v, id);
    }

    // Query 10 times and verify recall >= 0.85 against brute force
    let mut total_overlap = 0;
    let k = 10;
    let num_queries = 10;

    for _ in 0..num_queries {
        let query = random_vec(&mut rng);
        let hnsw_hits = index.search(&query, k);
        let hnsw_set: HashSet<usize> = hnsw_hits.into_iter().map(|(_, id)| id).collect();

        let mut bf: Vec<(f32, usize)> = all_vectors
            .iter()
            .map(|(id, v)| (DistanceMetric::Cosine.compute(&query, v), *id))
            .collect();
        bf.sort_by(|a, b| a.0.total_cmp(&b.0));
        let bf_set: HashSet<usize> = bf.into_iter().take(k).map(|(_, id)| id).collect();

        total_overlap += hnsw_set.intersection(&bf_set).count();
    }

    let avg_recall = total_overlap as f64 / (num_queries * k) as f64;
    assert!(
        avg_recall >= 0.85,
        "HNSW average recall {avg_recall} must be >= 0.85"
    );
}
