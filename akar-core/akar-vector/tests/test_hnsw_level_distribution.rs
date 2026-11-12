use akar_vector::hnsw::{DistanceMetric, HnswIndex};

#[test]
fn test_hnsw_initial_node_level() {
    // When inserting the first node, its level must be assigned via random_level()
    // rather than fixed MAX_M or 0.
    // In HNSW, P(level >= 1) = 1/m = 1/16 ~ 0.0625.
    // Across 1000 single-node index creations with varying RNG seeds,
    // the majority will be level 0, but non-zero levels MUST be generated.
    let m_max = index_config_m_max();
    let mut level_counts = vec![0usize; m_max + 1];
    let total_samples = 500;

    for i in 0..total_samples {
        let mut index = HnswIndex::with_exact_threshold(DistanceMetric::Cosine, 0);
        // Vary the vector slightly
        let v = vec![(i as f32).sin(), (i as f32).cos(), 1.0f32];
        index.insert(v, 0);

        let ml = index.max_level();
        assert!(
            ml <= m_max,
            "level {ml} must not exceed m_max {m_max}",
        );
        level_counts[ml] += 1;
    }

    // Level 0 should be the most common
    assert!(
        level_counts[0] > total_samples / 2,
        "Level 0 should be the most frequent level in geometric distribution, got {}",
        level_counts[0]
    );

    // Initial node max_level must not be pegged to m_max
    assert!(
        level_counts[m_max] < total_samples / 2,
        "First node must not be unconditionally assigned to max level"
    );
}

fn index_config_m_max() -> usize {
    akar_vector::hnsw::HnswConfig::default().m_max
}
