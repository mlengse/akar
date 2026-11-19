use akar_common::memory::MemoryManager;
use akar_storage::buffer_manager::{BufferManager, BufferManagerConfig};
use akar_storage::vector_index::VectorIndexTable;
use akar_vector::DistanceMetric;
use akar_vector::hnsw::HnswIndex;
use std::path::Path;
use std::sync::Arc;

fn setup_bm(db_path: &Path) -> BufferManager {
    let mm = Arc::new(MemoryManager::new(64 * 1024 * 1024));
    BufferManager::new(db_path.to_path_buf(), mm, BufferManagerConfig::default())
}

#[test]
fn test_vector_index_graph_persistence_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut bm = setup_bm(dir.path());

    let mut idx = VectorIndexTable::new(
        1,
        "graph_idx".into(),
        "items".into(),
        "embedding".into(),
        DistanceMetric::Cosine,
        4,
    );
    idx.register_file(&mut bm, dir.path());

    // Configure HnswIndex with exact_threshold = 0 to force graph construction
    idx.hnsw = HnswIndex::with_exact_threshold(DistanceMetric::Cosine, 0);

    // Insert 50 vectors with 4 dimensions
    for i in 0..50 {
        let v = vec![
            (i as f32 * 0.1).sin(),
            (i as f32 * 0.2).cos(),
            (i as f32 * 0.3).sin(),
            (i as f32 * 0.4).cos(),
        ];
        idx.hnsw_mut().insert(v, i);
    }

    assert_eq!(idx.hnsw().len(), 50);
    assert!(idx.hnsw().is_graph_built(), "graph must be built when threshold is 0");
    assert!(idx.hnsw().entry_point().is_some());

    // Save with graph persistence
    idx.save(&mut bm).expect("save must succeed");

    // Load into a fresh VectorIndexTable instance
    let mut loaded = VectorIndexTable::new(
        1,
        "graph_idx".into(),
        "items".into(),
        "embedding".into(),
        DistanceMetric::Cosine,
        0,
    );
    loaded.register_file(&mut bm, dir.path());
    loaded.load(&mut bm).expect("load must succeed");

    // Verify instant warmup without rebuilding: graph is already built
    assert_eq!(loaded.hnsw().len(), 50);
    assert!(
        loaded.hnsw().is_graph_built(),
        "reloaded index must have graph_built == true"
    );
    assert_eq!(loaded.hnsw().entry_point(), idx.hnsw().entry_point());
    assert_eq!(loaded.hnsw().max_level(), idx.hnsw().max_level());

    // Verify search parity
    let query = vec![0.5f32, -0.2f32, 0.8f32, 0.1f32];
    let original_results = idx.hnsw().search(&query, 5);
    let loaded_results = loaded.hnsw().search(&query, 5);

    assert_eq!(original_results.len(), 5);
    assert_eq!(loaded_results.len(), 5);
    for (orig, reloaded) in original_results.iter().zip(loaded_results.iter()) {
        assert_eq!(orig.1, reloaded.1, "node IDs must match");
        assert!((orig.0 - reloaded.0).abs() < 1e-5, "distances must match");
    }
}

#[test]
fn test_vector_index_flat_store_persistence_below_threshold() {
    let dir = tempfile::tempdir().unwrap();
    let mut bm = setup_bm(dir.path());

    let mut idx = VectorIndexTable::new(
        2,
        "flat_idx".into(),
        "items".into(),
        "embedding".into(),
        DistanceMetric::Euclidean,
        3,
    );
    idx.register_file(&mut bm, dir.path());

    // Default exact_threshold = 50_000, so 10 vectors stay flat
    for i in 0..10 {
        idx.hnsw_mut().insert(vec![i as f32, (i * 2) as f32, (i * 3) as f32], i);
    }

    assert_eq!(idx.hnsw().len(), 10);
    assert!(!idx.hnsw().is_graph_built(), "must remain flat below threshold");
    idx.save(&mut bm).expect("save flat index must succeed");

    let mut loaded = VectorIndexTable::new(
        2,
        "flat_idx".into(),
        "items".into(),
        "embedding".into(),
        DistanceMetric::Euclidean,
        0,
    );
    loaded.register_file(&mut bm, dir.path());
    loaded.load(&mut bm).expect("load flat index must succeed");

    assert_eq!(loaded.hnsw().len(), 10);
    assert!(!loaded.hnsw().is_graph_built(), "reloaded index must remain flat");

    let query = vec![2.0f32, 4.0f32, 6.0f32];
    let hits = loaded.hnsw().search(&query, 3);
    assert_eq!(hits[0].1, 2, "nearest neighbor must match");
}
