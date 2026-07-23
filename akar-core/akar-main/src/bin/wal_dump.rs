/// WAL dump tool — inspect the contents of a Akar write-ahead log.
///
/// Usage:
///   cargo run --bin wal_dump -- <database_path>
///
/// Reads `<database_path>/wal.log`, deserialises every record, and prints a
/// human-readable summary of each record to stdout.
use akar_storage::wal::{WAL, WALRecord};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: wal_dump <database_path>");
        std::process::exit(1);
    }

    let db_path = PathBuf::from(&args[1]);
    let wal_path = if db_path.to_string_lossy() == ":memory:" {
        eprintln!("Cannot dump WAL for in-memory database.");
        std::process::exit(1);
    } else {
        db_path.join("wal.log")
    };

    if !wal_path.exists() {
        eprintln!("WAL file not found: {}", wal_path.display());
        std::process::exit(1);
    }

    let metadata = match std::fs::metadata(&wal_path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Cannot read WAL metadata: {e}");
            std::process::exit(1);
        }
    };

    println!("WAL file: {}", wal_path.display());
    println!("File size: {} bytes", metadata.len());
    println!("---");

    let mut wal = WAL::new(wal_path);
    if let Err(e) = wal.load_from_disk() {
        eprintln!("Failed to load WAL: {e}");
        std::process::exit(1);
    }

    let records = wal.records();
    if records.is_empty() {
        println!("(no records)");
        return;
    }

    for (i, record) in records.iter().enumerate() {
        let tag_byte = tag_for_record(record);
        println!("[{:3}] {}  {}", i + 1, tag_byte, record);
    }

    println!("---");
    println!("Total records: {}", records.len());
    println!("Total size:    {} bytes", wal.total_size());
}

fn tag_for_record(record: &WALRecord) -> char {
    match record {
        WALRecord::Insert { .. } => 'I',
        WALRecord::Delete { .. } => 'D',
        WALRecord::Update { .. } => 'U',
        WALRecord::UpdateFsm { .. } => 'F',
        WALRecord::ColumnWrite { .. } => 'W',
        WALRecord::LocalWALData { .. } => 'L',
        WALRecord::Commit { .. } => 'C',
        WALRecord::Rollback { .. } => 'R',
        WALRecord::Checkpoint => 'K',
        WALRecord::CreateTable { .. } => 'T',
        WALRecord::DropTable { .. } => 'A',
        WALRecord::AlterTable { .. } => 'M',
        WALRecord::CreateIndex { .. } => 'N',
        WALRecord::DropIndex { .. } => 'X',
        WALRecord::CreateSequence { .. } => 'Q',
    }
}
