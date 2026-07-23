# Evaluasi Implementasi "Auto-Server" untuk Kuzu Rust 17/07/2026

Berdasarkan analisis pada *codebase* `akar-core` dan direktori *bindings* Rust (`tools/rust_api`), implementasi mode "Auto-Server" atau "Micro-Server" untuk Kuzu di lingkungan Rust adalah **sangat mungkin dan layak dilakukan**.

Berikut adalah rincian analisis arsitektur, tantangan, dan rekomendasi pendekatannya.

## 1. Kondisi Codebase Saat Ini

Kuzu menyediakan *native Rust bindings* yang terletak di `tools/rust_api` yang membungkus *crate* `akar-main` dan `akar-common`. 
File `src/native.rs` menunjukkan bahwa Rust saat ini hanya bertindak sebagai *wrapper* tipis (FFI) untuk objek inti C++:
*   `Database`: Membungkus `RawDatabase` dari Kuzu C++.
*   `Connection`: Membungkus koneksi untuk mengeksekusi `query` dan `prepare`.

Saat ini, konkurensi diatur sepenuhnya oleh *engine* C++ Kuzu, yang berarti arsitekturnya mengandalkan _Single-Writer, Multiple-Reader_ (SWMR) di level file.

## 2. Kelayakan "Auto-Server" (Sidecar Pattern)

Untuk membuat Kuzu Rust menangani konkurensi lintas-proses (*multi-process multi-writer*), kita dapat membangun arsitektur **Auto-Server** langsung di dalam *crate* Rust.

### Keuntungan
- **Keamanan Data**: Mengeliminasi risiko *file corruption* karena hanya ada satu proses Rust utama yang menahan *exclusive lock* pada file database.
- **Transparan**: Proses eksternal (*client*) merasa seperti berkomunikasi langsung dengan database, padahal mereka berbicara dengan *micro-server* Rust.
- **Ekosistem Rust**: Kita bisa memanfaatkan `Tokio` (untuk *async runtime* dan *channels*) serta `Tonic` (untuk komunikasi gRPC/IPC antar proses) yang sudah terbukti andal.

## 3. Strategi Implementasi (Rekomendasi)

Implementasi ini tidak perlu merombak kode C++ Kuzu sama sekali. Semuanya bisa dilakukan sepenuhnya di lapisan Rust dengan menambahkan fitur opsional (misalnya `feature = "server"`) di `Cargo.toml`.

### Komponen yang Diperlukan:
1. **`KuzuServer` (Manajer Utama)**
   Sebuah *struct* Rust yang mengambil kepemilikan (`ownership`) eksklusif atas `Database`. Struct ini akan menjalankan *Tokio task* di *background* untuk mendengarkan koneksi masuk.
2. **IPC / TCP Layer (Komunikasi Lintas Proses)**
   Menggunakan **Tonic (gRPC)** atau Unix Domain Sockets (via *crate* `interprocess` / `tokio::net`) agar proses eksternal bisa mengirim *query string* dan parameter ke *server*.
3. **`ConnectionPool` & `MPSC Channel` (Pengatur Antrean Write)**
   - Semua *query read* (SELECT, MATCH) bisa didistribusikan ke banyak koneksi pembaca secara paralel (*multi-threading* yang sudah didukung Kuzu).
   - Semua *query write* (CREATE, MERGE, SET) akan dimasukkan ke dalam satu *channel* antrean (`mpsc`), sehingga dieksekusi secara sekuensial oleh satu *thread* penulis khusus. Ini menjamin konkurensi *write* aman tanpa me-reject proses client eksternal.
4. **`KuzuClient` (Pengganti Connection)**
   Aplikasi klien Rust lainnya tidak lagi memanggil `Database::new`, melainkan menggunakan objek `KuzuClient` yang di balik layar mengirim *query* ke server melalui IPC/TCP.

## 4. Evaluasi Kompleksitas

| Aspek | Tingkat Kesulitan | Keterangan |
| :--- | :--- | :--- |
| **Modifikasi Core C++** | **Rendah (Nihil)** | Tidak ada perubahan yang dibutuhkan di level C++ Kuzu. |
| **Integrasi Rust (Tokio/Tonic)** | **Menengah** | Membutuhkan pengalaman dengan *async Rust*, gRPC/Protobuf, atau IPC. |
| **Manajemen State (Locking)** | **Mudah** | Mengandalkan *mutex* atau *channels* standar bawaan Rust. |
| **Serialisasi Hasil (QueryResult)**| **Tinggi** | Kuzu mengembalikan objek graf (Nodes, Edges) dalam format internal. Mengonversi hasil ini ke format *stream* (JSON atau Protobuf) untuk dikirim ke *client* eksternal membutuhkan *mapping* data yang teliti.

## Kesimpulan

Mode "Auto-Server" adalah solusi paling elegan untuk masalah *concurrent multi-writer* antar proses di Kuzu. Mengembangkan layer ini secara eksklusif di dalam `akar-rust` adalah **ide yang sangat solid**. Ini tidak merusak desain memori Kuzu yang mengutamakan performa, sambil memberikan fleksibilitas akses seperti database *client-server* tradisional kepada pengguna ekosistem Rust.
