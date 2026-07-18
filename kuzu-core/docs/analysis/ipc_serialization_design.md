# Desain Arsitektur: Komunikasi & Serialisasi untuk Kuzu Rust (Auto-Server) 17/07/2026

Karena proyek ini adalah *porting* murni ke ekosistem Rust (menggabungkan Kuzu & Ladybug), kita memiliki kebebasan penuh untuk merancang lapisan komunikasi antar-proses (IPC) yang paling efisien tanpa terikat oleh *legacy* C++.

Tujuan utama dari arsitektur ini adalah meminimalkan *overhead* serialisasi (menyalin data dari RAM ke jaringan/socket) dan mendukung pengiriman data graf berukuran masif (jutaan node/edge) secara mulus.

---

## 1. Protokol Komunikasi (Communication Protocol)

Untuk menjembatani proses *Client* dan *Auto-Server* (Daemon Kuzu Rust), protokol yang dipilih harus mendukung performa lokal (IPC) namun tetap bisa diskalakan ke jaringan jika diperlukan nanti.

### Rekomendasi Utama: **gRPC di atas Unix Domain Sockets (UDS) / Named Pipes**
Alih-alih menggunakan TCP biasa (`127.0.0.1`), kita menjalankan gRPC di atas UDS (Linux/Mac) atau Named Pipes (Windows). 
*   **Mengapa?** gRPC over UDS memotong *overhead* routing TCP/IP dari OS kernel. Datanya langsung disalin dari memori OS ke memori OS lainnya.
*   **Implementasi Rust:** Menggunakan kombinasi _crate_ `tonic` (gRPC) dan `hyper` dengan konektor UDS.
*   **Interoperabilitas:** Jika suatu saat Anda ingin membuat *driver* Python atau Node.js untuk Kuzu Rust ini, mereka bisa langsung terkoneksi menggunakan standar gRPC standar tanpa perlu menulis parser khusus.

### Alternatif Khusus Analitik: **Apache Arrow Flight**
Jika `Kuzu Rust` akan sering mengembalikan data berbentuk tabular/kolom (*Node tables, Edge properties*) dalam ukuran bergiga-giga, **Arrow Flight RPC** adalah standar emas saat ini.
*   *Arrow Flight* juga dibangun di atas gRPC, tetapi dirancang agar transfer data kolom terjadi dengan **zero-copy** (data tidak perlu diserialisasi ulang ke format teks atau Protobuf).

---

## 2. Serialisasi Hasil (Result Serialization)

Bagaimana kita mengemas data struktur Graf (Node, Edge, Properties) menjadi bentuk byte yang dikirim via Socket? Ini adalah bagian paling krusial. Graph database mengembalikan struktur bersarang (*nested/recursive*) yang sulit dikemas oleh format biasa seperti JSON.

### Rekomendasi: **FlatBuffers atau Cap'n Proto (Zero-Copy Serialization)**
Untuk struktur graf, Protobuf biasa agak lambat karena mewajibkan tahap alokasi memori (dekode/enkode) di sisi klien. 
*   **FlatBuffers** (atau Cap'n Proto) memungkinkan proses *Client* untuk langsung membaca struktur graf dari memori *buffer* mentah (*byte array*) yang diterima dari soket, **tanpa mem-parsingnya terlebih dahulu**.
*   **Contoh Kasus:** Ketika Kuzu Rust Server mengirimkan 1000 `Node`, klien langsung memiliki *pointer* ke *array* tersebut di memori lokalnya.

### Schema Desain (Contoh Representasi Protobuf/FlatBuffers)
```protobuf
// Pesan dikirim sebagai stream
message QueryResultChunk {
  repeated Node nodes = 1;
  repeated Edge edges = 2;
  // Representasi kolom nilai/properti
  map<string, bytes> properties = 3; 
}

message Node {
  int64 internal_id = 1;
  int32 label_id = 2;
}

message Edge {
  int64 src_id = 1;
  int64 dst_id = 2;
  int32 label_id = 3;
}
```

---

## 3. Format Stream (Streaming Format)

Permintaan *query* (seperti `MATCH (a)-[e]->(b) RETURN a, e`) bisa menghasilkan ratusan juta baris. Memaksa server memuat semuanya ke RAM lalu diserialisasi akan menyebabkan *Out of Memory* (OOM).

Oleh karena itu, komunikasi harus berjalan secara **Asynchronous Streaming**.

### Pola: *Server-Side Streaming RPC*
Di dalam Rust (via Tonic), *endpoint* akan didefinisikan sebagai *stream*:

```rust
// Di sisi Server (Tonic Service)
async fn execute_query(
    &self,
    request: Request<QueryRequest>,
) -> Result<Response<Self::ExecuteQueryStream>, Status> {
    
    let (tx, rx) = tokio::sync::mpsc::channel(128); // Buffer channel
    
    tokio::spawn(async move {
        // Eksekusi core Kuzu-Rust
        let result_iterator = kuzu_core::execute_match(...);
        
        // Baca dalam chunk (misal 1024 hasil per batch)
        for chunk in result_iterator.chunks(1024) {
            let serialized_chunk = serialize_to_flatbuffers(chunk);
            tx.send(Ok(serialized_chunk)).await.unwrap();
        }
    });

    Ok(Response::new(ReceiverStream::new(rx))) // Stream langsung ke klien
}
```

### Bagaimana Format Chunking Bekerja?
1. **Batching:** Jangan mengirim 1 Node/Edge per pesan. Kumpulkan menjadi bentuk matriks/vektor berukuran *batch* (misal 1024 elemen). Kuzu secara internal juga berbasis *Vectorized Execution*, jadi ukuran *batch* ini bisa mengikuti ukuran *DataChunk* internal Kuzu.
2. **Backpressure:** Dengan menggunakan `mpsc::channel` milik Tokio (yang memiliki batas kapasitas, misal `128`), Kuzu Server secara otomatis akan berhenti sementara (Pause) membaca dari disk jika Klien Rust membaca terlalu lambat. Ini mencegah memori server meledak.
3. **Pipelining:** Sementara *Client* sedang melakukan deserialisasi terhadap *Chunk* 1, *Server* sudah mengambil *Chunk* 2 dari disk dan mengirimkannya melalui soket.

---

## Rangkuman Integrasi Arsitektur
Jika Anda men-porting Kuzu murni ke Rust, Anda memiliki tumpukan teknologi modern yang sangat kuat:
*   **Storage & Execution:** Kuzu-Rust Core (Vectorized).
*   **Concurrency Manager:** `tokio::sync::RwLock` dan `mpsc` untuk mengatur multi-writer.
*   **Transport Layer:** Unix Domain Sockets (Linux/Mac) atau Named Pipes (Windows).
*   **RPC Framework:** `tonic` (gRPC).
*   **Data Serialization:** Apache Arrow IPC (untuk kolom tabular) atau FlatBuffers (untuk topologi relasi).
