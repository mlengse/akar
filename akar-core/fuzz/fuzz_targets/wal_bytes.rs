#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz WAL deserialization from arbitrary bytes
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("fuzz_wal.log");
    std::fs::write(&wal_path, data).unwrap();

    let mut wal = akar_storage::wal::WAL::new(wal_path.clone());
    let _ = wal.load_from_disk();

    // Replay any successfully loaded records — must not panic
    let _ = wal.replay(|record| {
        let _ = format!("{:?}", record);
        Ok(())
    });

    // Also test the WALReplayer path
    let _ = akar_storage::wal_replayer::WALReplayer::replay(&wal_path, |record| {
        let _ = format!("{:?}", record);
        Ok(())
    });
});
