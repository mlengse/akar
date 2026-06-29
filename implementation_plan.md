# Rencana Implementasi Fitur Unggulan LadybugDB ke Kuzu Rust (kuzu-core)

Dokumen ini membandingkan basis kode **LadybugDB** (C++) dengan porting Rust **Kuzu Core** (`kuzu-core`), mendaftar fitur unggulan LadybugDB yang belum ada di Kuzu Rust, serta merancang rencana implementasi terperinci untuk mengadopsi fitur-fitur tersebut ke dalam Kuzu Rust.

---

## 1. Analisis Perbandingan Codebase

Berikut adalah peta perbandingan arsitektur antara tiga varian evolusi Kuzu:

| Dimensi | **LadybugDB (C++ Fork)** | **Kuzu (Vela Partners C++ Fork)** | **Kuzu Core (Pure Rust Port)** |
|---|---|---|---|
| **Fokus Utama** | Efisiensi graf analitis lokal, AI Agent memory, HNSW native | Multi-Agent concurrency, stabilitas penulisan paralel | Re-implementasi penuh Kuzu ke Rust tanpa dependensi C++ |
| **Model Transaksi** | *Single-Writer Constraint* (tradisional ACID) | **Concurrent Multi-Writer Support** (paralel writes) | *Single-Writer Constraint* (MVCC tradisional di Rust) |
| **Indeks Vektor** | Native HNSW terintegrasi di dalam graf & query engine | Sama dengan upstream Kuzu (ekstensi terpisah) | Pustaka HNSW in-memory (`kuzu-vector`), belum terintegrasi |
| **Indeks PK** | Mendukung **HASH** dan **ART** (Adaptive Radix Tree) | Mendukung HASH | Baru mendukung **HASH** saja |
| **Manajemen Memori** | Spilling ke disk & *batch stream-merge* di Arrow-CSR | Pengelolaan antrean transaksi C++ | In-memory buffer dengan buffer manager standar |
| **Interoperabilitas** | C++ native dengan binding eksternal luas | C++ native dengan fokus ke Python/Vela | Rust native murni dengan FFI backward-compatible |

---

## 2. Fitur Unggulan LadybugDB yang Belum Ada di Kuzu Rust

Berdasarkan analisis repositori `ladybug`, berikut adalah fitur-fitur unggulan yang belum diintegrasikan ke dalam porting Rust `kuzu-core`:

### A. Indeks ART (Adaptive Radix Tree) untuk Primary Key
*   **Deskripsi:** Radix tree adaptif berbasis byte-ordered keys yang menggantikan atau berjalan paralel dengan `HashIndex`.
*   **Keunggulan:** Mendukung pencarian rentang (*range scans*) pada primary key (misal: `p.ID >= 10 AND p.ID < 20`), pemulihan crash (*rollback cleanup*), checkpointing terstruktur, dan efisiensi traversal tinggi.
*   **Status Kuzu Rust:** Hanya memiliki `HashIndex` (`kuzu-storage/src/index.rs`) yang tidak mendukung range scan (hanya equality lookup).

### B. Indeks Vektor Native HNSW yang Terintegrasi Penuh
*   **Deskripsi:** Indeks HNSW (Hierarchical Navigable Small World) yang terhubung langsung dengan catalog, storage manager, parser Cypher, optimizer, dan processor.
*   **Keunggulan:** Mengeksekusi pencarian kemiripan vektor (*vector similarity search*) secara hibrida bersanding dengan traversal graf analitis dalam satu kueri Cypher.
*   **Status Kuzu Rust:** Crate `kuzu-vector` baru menyediakan implementasi algoritma HNSW in-memory (`hnsw.rs`) dan fungsi skalar jarak (`lib.rs`), tetapi belum memiliki parser Cypher `CREATE INDEX` khusus vektor, registrasi index type di catalog, maupun integrasi dengan executor pipeline.

### C. Manajemen Memori: Arrow-CSR Spilling & Stream-Merge
*   **Deskripsi:** Mekanisme pengontrolan lonjakan memori transien (*transient peak memory*) menggunakan `Spiller` yang memindahkan *sorted runs* data ke disk saat batch insert melewati batas memori, kemudian digabungkan secara streaming (*stream-merge*).
*   **Keunggulan:** Menjaga performa tetap stabil di mesin berspesifikasi rendah/RAM terbatas saat melakukan `COPY FROM` dataset graf raksasa.
*   **Status Kuzu Rust:** Belum memiliki mekanisme spilling ke disk di `ColumnChunk` atau `NodeGroup` selama batch load/DML.

---

## 3. Rencana Implementasi untuk Kuzu Rust (kuzu-core)

Kami mengusulkan implementasi bertahap untuk membawa ketiga fitur unggulan ini ke `kuzu-core`.

### FASE 1: Porting ART Primary Key Index
Fase ini berfokus pada penambahan struktur indeks radix tree adaptif ke `kuzu-storage` dan integrasinya dengan parser, catalog, dan optimizer.

#### [NEW] [art_index.rs](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-storage/src/art_index.rs)
Membuat struktur data radix tree adaptif di `kuzu-storage`:
*   Definisikan tipe node: `Node4`, `Node16`, `Node48`, `Node256`.
*   Implementasikan key encoding order-preserving (mengubah data numerik, string, dll. menjadi byte array yang melestarikan urutan sorting).
*   Implementasikan operasi: `insert`, `lookup`, `delete`, `range_scan` (mengambil row ID dalam rentang key minimum/maximum).
*   Integrasikan dengan `checkpoint` untuk menulis node-node ART ke disk dan melakukan rekonstruksi cepat saat startup.

#### [MODIFY] [lib.rs](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-storage/src/lib.rs)
*   Ekspos modul baru `art_index` dan struktur `ArtPrimaryKeyIndex`.

#### [MODIFY] [catalog](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-catalog/src/lib.rs)
*   Tambahkan enum `IndexType` (`Hash`, `Art`) ke `NodeTableEntry` agar database mencatat jenis indeks primary key yang digunakan.

#### [MODIFY] [parser & binder](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-parser/src/lib.rs)
*   Parser: Dukung sintaks Cypher `CREATE ART INDEX index_name FOR (p:Person) ON (p.ID)`.
*   Binder: Tambahkan logika resolusi tipe indeks untuk mendeteksi indeks ART dan memvalidasi kecocokan kolom primary key.

#### [MODIFY] [optimizer & processor](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-optimizer/src/lib.rs)
*   Filter Push-down Optimizer: Jika terdeteksi filter ketidaksamaan (misal `p.age > 25`) pada kolom indeks ART, ubah physical plan untuk menggunakan operator baru `ArtIndexRangeScan` alih-alih `FullScan + Filter`.
*   Physical Processor: Buat operator `ArtIndexRangeScan` yang mengambil baris menggunakan fungsi `range_scan` dari indeks ART.

---

### FASE 2: Integrasi Penuh HNSW Vector Index
Fase ini mengintegrasikan indeks HNSW yang sudah ada di `kuzu-vector` ke dalam mesin penyimpanan dan query database.

#### [MODIFY] [kuzu-vector](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-vector/src/lib.rs)
*   Ubah `HnswIndex` agar dapat terhubung dengan `BufferManager` dari `kuzu-storage` untuk menulis data graf HNSW ke disk secara ter-page.
*   Implementasikan serialisasi dan deserialisasi state HNSW ke page-page biner database.

#### [MODIFY] [catalog & storage](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-catalog/src/lib.rs)
*   Dukung pendaftaran tipe indeks `HNSW` di catalog graf.
*   Integrasikan `HnswIndex` ke dalam `NodeTable` di `kuzu-storage` agar saat terjadi operasi penulisan node baru yang memiliki properti embedding vektor, nilai vektor tersebut otomatis dimasukkan ke indeks HNSW.

#### [MODIFY] [parser, binder, & processor]
*   Parser: Dukung `CREATE HNSW INDEX vector_idx FOR (p:Person) ON (p.embedding)`.
*   Processor: Buat physical operator `VectorSimilarityScan` yang memanggil fungsi pencarian `search` HNSW untuk mendapatkan node dengan kecocokan semantik tertinggi, memotong eksekusi full-scan pada dataset besar.

---

### FASE 3: Implementasi Disk Spilling & Stream-Merge (Arrow-CSR)
Fase ini mengoptimalkan penulisan batch besar dengan menghemat konsumsi RAM melalui disk spilling.

#### [NEW] [spiller.rs](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-storage/src/spiller.rs)
*   Implementasikan struktur `Spiller` yang memantau alokasi memori buffer selama proses batch insert.
*   Jika ukuran data batch melampaui threshold (misal 8GB atau batas RAM fisik):
    1. Sortir elemen di memori berdasarkan kunci.
    2. Tulis sebagai berkas *sorted run* terkompresi di folder temporer database.
    3. Kosongkan buffer memori.

#### [MODIFY] [column_chunk.rs & node_group.rs](file:///c:/Users/anjan/dev/memory/kuzu/kuzu-core/kuzu-storage/src/column_chunk.rs)
*   Integrasikan `Spiller` ke dalam operasi penyisipan massal (`COPY FROM` / batch insert).
*   Di akhir penyisipan (fase `finalize`), jika terdapat berkas *spilled runs* di disk, gunakan algoritma **Multi-way Stream Merge** untuk membaca setiap run secara teraliri (*streaming*), mendeteksi duplikasi primary key lintas run, dan menulis graf CSR final ke penyimpanan utama secara berurutan.

---

## 4. Verification Plan

Untuk memastikan implementasi berjalan dengan benar tanpa merusak fungsi yang sudah ada:

### Automated Tests
*   **Unit Tests Baru di `kuzu-storage`:**
    *   Pengujian key encoding ART untuk semua tipe data (`Int64`, `Float`, `String`, dll.) untuk memastikan byte-ordering yang konsisten.
    *   Pengujian insert, update, delete, dan `range_scan` pada `ArtPrimaryKeyIndex` dengan berbagai ukuran dataset.
    *   Pengujian ketahanan `Spiller` dalam membagi, menulis, dan melakukan stream-merge pada data yang melampaui threshold.
*   **Integration Tests Baru di `kuzu-main`:**
    *   Kueri Cypher range search: `MATCH (p:Person) WHERE p.ID >= 100 AND p.ID <= 200 RETURN p.name` dengan indeks HASH vs ART.
    *   Pengujian pembuatan dan pencarian kemiripan vektor secara hibrida: `MATCH (p:Person) WHERE cosine_similarity(p.embedding, $query) > 0.8 RETURN p.name`.
    *   Pengujian stress-test batch `COPY FROM` berukuran gigabyte dengan threshold memori yang sengaja diset sangat rendah (misal 50MB) untuk memaksa spilling ke disk dan memvalidasi proses stream-merge.

### Manual Verification
*   Validasi kompatibilitas silang platform (Windows, Linux) dengan melakukan build workspace lengkap di PowerShell (`cargo build --workspace`).
*   Verifikasi performa memori transien lewat logging statistik RAM selama eksekusi batch loading data graf berukuran besar.
