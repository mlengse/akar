use akar_vector::hnsw::{DistanceMetric, HnswConfig, HnswIndex};
use std::collections::HashSet;

#[test]
fn test_hnsw_alpha_recall_clustered_embeddings() {
    let dim = 16;
    let num_clusters = 4;
    let points_per_cluster = 75;
    let total_points = num_clusters * points_per_cluster;

    // Generate cluster centers
    let mut cluster_centers = Vec::new();
    for c in 0..num_clusters {
        let mut center = vec![0.0f32; dim];
        center[c % dim] = 1.0;
        center[(c + 1) % dim] = 0.5;
        let norm: f32 = center.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut center {
            *x /= norm;
        }
        cluster_centers.push(center);
    }

    // Generate tightly clustered points around centers
    let mut all_vectors = Vec::with_capacity(total_points);
    for c in 0..num_clusters {
        let center = &cluster_centers[c];
        for p in 0..points_per_cluster {
            let mut v = center.clone();
            // Add slight perturbation
            let noise = (p as f32 * 0.05).sin() * 0.08;
            v[p % dim] += noise;
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            for x in &mut v {
                *x /= norm;
            }
            all_vectors.push(v);
        }
    }

    // Build HNSW index with alpha = 1.2 relaxation
    let config = HnswConfig::with_alpha(16, 32, 256, 150, 1.2).expect("valid config");
    let mut index = HnswIndex::with_config(DistanceMetric::Cosine, 0, config);

    for (id, vec) in all_vectors.iter().enumerate() {
        index.insert(vec.clone(), id);
    }

    // Evaluate top-k recall across multiple queries
    let k = 5;
    let mut total_hits = 0;
    let num_queries = 20;

    for q_idx in 0..num_queries {
        let query_vec = &all_vectors[q_idx * 15 % total_points];

        // Ground truth exact search (brute force)
        let mut ground_truth: Vec<(f32, usize)> = all_vectors
            .iter()
            .enumerate()
            .map(|(id, vec)| (DistanceMetric::Cosine.compute(vec, query_vec), id))
            .collect();
        ground_truth.sort_by(|a, b| a.0.total_cmp(&b.0));
        let exact_top_k: HashSet<usize> = ground_truth.iter().take(k).map(|(_, id)| *id).collect();

        // HNSW search
        let hnsw_results = index.search(query_vec, k);
        let hnsw_top_k: HashSet<usize> = hnsw_results.iter().map(|(_, id)| *id).collect();

        let hits = exact_top_k.intersection(&hnsw_top_k).count();
        total_hits += hits;
    }

    let recall = total_hits as f32 / (num_queries * k) as f32;
    println!("Measured HNSW recall with alpha=1.2: {:.2}%", recall * 100.0);
    assert!(
        recall >= 0.95,
        "Recall with alpha=1.2 must achieve >= 95%, got {:.2}%",
        recall * 100.0
    );
}
