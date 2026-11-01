use akar_vector::hnsw::{DistanceMetric, HnswConfig, HnswIndex};
use std::collections::HashSet;

#[test]
fn test_hnsw_config_default() {
    let cfg = HnswConfig::default();
    assert_eq!(cfg.m, 16);
    assert_eq!(cfg.m_max, 32);
    assert_eq!(cfg.ef_construction, 512);
    assert_eq!(cfg.ef_search, 200);
}

#[test]
fn test_hnsw_config_high_recall() {
    let cfg = HnswConfig::high_recall();
    assert_eq!(cfg.m, 24);
    assert_eq!(cfg.m_max, 48);
    assert_eq!(cfg.ef_construction, 512);
    assert_eq!(cfg.ef_search, 300);
}

#[test]
fn test_hnsw_config_bounds_validation() {
    // Valid configuration
    assert!(HnswConfig::new(16, 32, 512, 200).is_ok());

    // m < 2
    let err = HnswConfig::new(1, 32, 512, 200).unwrap_err();
    assert_eq!(err, "m must be >= 2");

    // m_max < m
    let err = HnswConfig::new(16, 15, 512, 200).unwrap_err();
    assert_eq!(err, "m_max must be >= m");

    // ef_construction < m
    let err = HnswConfig::new(16, 32, 10, 200).unwrap_err();
    assert_eq!(err, "ef_construction must be >= m");

    // ef_search < 1
    let err = HnswConfig::new(16, 32, 512, 0).unwrap_err();
    assert_eq!(err, "ef_search must be >= 1");
}

#[test]
fn test_hnsw_index_with_config_compatibility() {
    let custom_cfg = HnswConfig::new(20, 40, 256, 150).expect("valid config");
    let mut index = HnswIndex::with_config(DistanceMetric::Cosine, 0, custom_cfg);

    assert_eq!(index.config(), &custom_cfg);
    assert_eq!(index.config().m, 20);
    assert_eq!(index.config().m_max, 40);
    assert_eq!(index.config().ef_construction, 256);
    assert_eq!(index.config().ef_search, 150);

    // Default constructors should carry Default config
    let idx_new = HnswIndex::new(DistanceMetric::Cosine);
    assert_eq!(idx_new.config(), &HnswConfig::default());

    let idx_thresh = HnswIndex::with_exact_threshold(DistanceMetric::Cosine, 100);
    assert_eq!(idx_thresh.config(), &HnswConfig::default());

    // Basic insert and search check with custom config
    index.insert(vec![1.0, 0.0, 0.0], 1);
    index.insert(vec![0.0, 1.0, 0.0], 2);
    index.insert(vec![0.0, 0.0, 1.0], 3);

    let hits = index.search(&[1.0, 0.1, 0.0], 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].1, 1);
}

#[test]
fn test_clustered_distribution_recall() {
    let num_vectors = 600;
    let dims = 64;
    let num_clusters = 12;
    let spread = 0.15f32;
    let mut rng: u64 = 123456789;

    let random_vec = |rng: &mut u64| -> Vec<f32> {
        let mut v = Vec::with_capacity(dims);
        for _ in 0..dims {
            *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
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

    let sample_clustered = |rng: &mut u64, centroid: &[f32]| -> Vec<f32> {
        let mut v = Vec::with_capacity(dims);
        let mut i = 0;
        while i < dims {
            *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let u1 = (((*rng >> 33) as f32 + 1.0) / (1u64 << 31) as f32).clamp(1e-7, 1.0);
            *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let u2 = (((*rng >> 33) as f32 + 1.0) / (1u64 << 31) as f32).clamp(1e-7, 1.0);
            let r = (-2.0 * u1.ln()).sqrt();
            let theta = 2.0 * std::f32::consts::PI * u2;
            let z0 = r * theta.cos();
            let z1 = r * theta.sin();

            v.push(centroid[i] + spread * z0);
            if i + 1 < dims {
                v.push(centroid[i + 1] + spread * z1);
            }
            i += 2;
        }
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        v
    };

    // Generate centroids
    let centroids: Vec<Vec<f32>> = (0..num_clusters).map(|_| random_vec(&mut rng)).collect();

    // Create index in graph mode (threshold 0)
    let config = HnswConfig::new(16, 32, 256, 100).expect("valid config");
    let mut index = HnswIndex::with_config(DistanceMetric::Cosine, 0, config);

    let mut all_vectors = Vec::with_capacity(num_vectors);
    for id in 0..num_vectors {
        let centroid = &centroids[id % centroids.len()];
        let v = sample_clustered(&mut rng, centroid);
        all_vectors.push((id, v.clone()));
        index.insert(v, id);
    }

    // Evaluate 10 test queries
    let k = 10;
    let num_queries = 10;
    let mut total_overlap = 0;

    for q_idx in 0..num_queries {
        let centroid = &centroids[q_idx % centroids.len()];
        let query = sample_clustered(&mut rng, centroid);

        let hits = index.search_with_ef(&query, k, 150);
        let hit_set: HashSet<usize> = hits.into_iter().map(|(_, id)| id).collect();

        let mut exact: Vec<(f32, usize)> = all_vectors
            .iter()
            .map(|(id, v)| (DistanceMetric::Cosine.compute(&query, v), *id))
            .collect();
        exact.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let exact_set: HashSet<usize> = exact.into_iter().take(k).map(|(_, id)| id).collect();

        total_overlap += hit_set.intersection(&exact_set).count();
    }

    let avg_overlap = total_overlap as f64 / (num_queries * k) as f64;
    assert!(
        avg_overlap >= 0.90,
        "Clustered distribution overlap {avg_overlap:.4} must be >= 0.90"
    );
}
