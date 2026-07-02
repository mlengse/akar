# Rencana Implementasi Lanjutan — Update Status & Prioritas

> **Tanggal:** 2026-07-02 (revisi 2 — ditemukan bug korektness fundamental saat mencoba mengerjakan Prioritas 1 lama)
> **Menggantikan status di:** `KONSOLIDASI_DOKUMEN.md` (ditulis 2026-07-01, sudah basi)
> **Metode verifikasi:** Pembacaan kode langsung + `git show`/`git log` + trace eksekusi empiris via `kuzu-core/.claude/skills/run-kuzu-cli/driver.mjs`, bukan asumsi dari dokumen lama.

---

## 0. 🔴🔴 UPDATE KRITIS (revisi 2) — Bug Korektness Fundamental, Bukan Sekadar "Belum Disambung"

Saat mulai mengerjakan **Prioritas 1 lama (wiring SIP)**, penelusuran mendalam terhadap cara `HashJoin` benar-benar dieksekusi menemukan **dua bug korektness bertingkat** yang jauh lebih mendasar daripada SIP. Ini sekarang menjadi prioritas tertinggi — SIP (optimasi performa) tidak ada gunanya dikerjakan di atas fondasi yang salah secara fungsional.

### 0.1 🔴 PALING KRITIS: `evaluate_property_access` mengabaikan nama properti sepenuhnya

**File:** `kuzu-processor/src/expression_evaluator.rs:158-166`

```rust
fn evaluate_property_access(&self, obj: &Expression, _prop: &str, chunk: &DataChunk) -> Result<ValueVector, String> {
    // Simplified: evaluate the object expression
    self.evaluate(obj, chunk)
}
```

Parameter `_prop` (nama properti seperti `"name"`/`"id"`) **tidak pernah dibaca**. Jadi `a.id`, `a.name`, `a.apa_pun` semua dievaluasi identik seolah hanya `a` yang ditulis.

**Dibuktikan langsung (reproducible):**
```
CREATE NODE TABLE T(id INT64, name STRING, PRIMARY KEY(id));
CREATE (:T {id: 1, name: 'alice'});
CREATE (:T {id: 2, name: 'bob'});
MATCH (t:T) WHERE t.name = 'bob' RETURN t.id;
→ (empty)   ← SALAH. Baris dengan name='bob' ada (id=2), harus 1 baris hasil.
```

**Dampak:** Ini fondasi evaluasi ekspresi yang dipakai di **WHERE filter, RETURN, kondisi JOIN, SET, argumen fungsi** — di mana pun properti diakses. Ini akar penyebab dari 3 gejala terpisah yang tampak sepanjang audit:
1. Bug "MATCH...RETURN p.property selalu kembalikan nilai PK" (didokumentasikan di `kuzu-core/.claude/skills/run-kuzu-cli/SKILL.md` Gotchas — sekarang jelas ini gejala dari bug yang sama, bukan bug terpisah).
2. Kondisi JOIN berbasis properti (lihat 0.2 di bawah) tidak benar-benar mengevaluasi properti yang dimaksud.
3. Kemungkinan besar WHERE filter berbasis properti **di seluruh sistem** tidak benar-benar memfilter sesuai propertinya (baru diverifikasi untuk kasus STRING equality; kasus INT literal seperti `WHERE t.id = 2` malah memicu bug parser terpisah — "Parse error: Int: invalid digit found in string" — perlu investigasi tambahan, kemungkinan tidak berkaitan langsung).

**Ini kandidat bug paling berdampak luas di seluruh basis kode.** Hampir semua query nyata memakai `variabel.properti` di WHERE atau RETURN.

**Estimasi perbaikan:** 2-4 hari. Perlu: (a) mekanisme resolusi nama properti → indeks kolom fisik menggunakan skema tabel (`TableCatalog`/`ColumnDefinition`) yang terikat ke variabel/tabel asal `obj`, (b) audit setiap tempat lain yang mengasumsikan `PropertyAccess` bisa diabaikan namanya, (c) test regresi luas untuk WHERE/RETURN/JOIN dengan properti non-PK.

### 0.2 🔴 KRITIS: Model eksekusi flat-pipeline menimpa (bukan mengakumulasi) chunk dari scan berurutan — HashJoin & CrossProduct pass-through tanpa join nyata

**File:** `kuzu-processor/src/processor.rs` — arm `LogicalOperator::ScanNode`/`ScanRel` (di dalam `execute()`)

Model eksekusi adalah pipeline **flat linear** di atas `&[LogicalOperator]` — dikonfirmasi via trace langsung (bukan asumsi):

```
CREATE NODE TABLE A(id INT64, name STRING, PRIMARY KEY(id));
CREATE NODE TABLE B(id INT64, val INT64, PRIMARY KEY(id));
CREATE (:A {id: 1, name: 'alpha'}); CREATE (:A {id: 2, name: 'beta'});
CREATE (:B {id: 999, val: 111});    CREATE (:B {id: 888, val: 222});
MATCH (a:A), (b:B) WHERE a.id = b.id RETURN a.id, b.val;
→ mengembalikan 2 baris (999,111) dan (888,222)
→ SALAH. A dan B tidak punya id yang sama sama sekali — join yang benar harus 0 baris.
```

Rencana tereksekusi (dikonfirmasi via debug trace langsung terhadap `optimized_plan`): `[Scan(A), Scan(B), HashJoin{join_keys:[a=b], build_side: dummy, probe_side: dummy}, Projection(a,b)]`.

- `Scan(A)` mengisi `intermediate_result`. `Scan(B)` **menimpanya** (bukan menambah) — data A hilang total.
- `HashJoin` menerima hanya 1 chunk (dari Scan B). `PhysicalHashJoin::execute` (`physical_operator.rs:1642`): `if input.len() < 2 { return Ok(input); }` → pass-through tanpa join.
- `Projection` membaca kolom B secara posisional, memberi label seolah itu `a.id`/`b.val`.
- **`PhysicalCrossProduct` (`physical_operator.rs:1158-1165`) punya bug arsitektur identik** (split input di tengah, `input.len() < 2` → pass-through) — artinya `MATCH (a:A), (b:B) RETURN ...` TANPA WHERE (cross product murni) kemungkinan juga rusak dengan cara yang sama.
- Catatan tambahan: `LogicalHashJoin.build_side`/`probe_side` (field pohon boks) **selalu berisi placeholder dummy** (`ScanNode{table_name:"join_build", table_id:0}`) dari `flatten_join_plan` (`kuzu-planner/src/join_order.rs:160-178`) — field ini secara desain tidak pernah dibaca oleh eksekusi flat (`_h` di match arm, unused).

**Dampak:** Setiap MATCH multi-pola dengan join berbasis WHERE (atau cross product tanpa WHERE) mengembalikan hasil yang salah secara diam-diam. Test session sebelumnya "terlihat benar" murni kebetulan karena data tes simetris (id sama di kedua tabel).

**Estimasi perbaikan:** 3-5 hari. Perlu: (a) ScanNode/ScanRel mengakumulasi (append) ke `intermediate_result`, bukan menimpa, ketika beberapa scan berurutan mengarah ke HashJoin/CrossProduct; (b) HashJoin menurunkan `build_columns`/`probe_columns` sungguhan dari `join_keys` + skema tabel (bukan hardcode index 0) — **ini bergantung pada 0.1 selesai dulu**, karena `join_keys` berisi ekspresi `PropertyAccess`/`Variable` yang perlu resolusi properti yang benar; (c) audit `PhysicalCrossProduct` dengan pola sama; (d) test regresi: join dengan ID tidak simetris, join 0-match, join 3-tabel.

### Urutan Pengerjaan yang Benar

**0.1 harus diperbaiki lebih dulu** — 0.2 (HashJoin) butuh resolusi properti yang benar untuk menentukan kolom join yang tepat. SIP (Prioritas 1 lama, sekarang di bawah) tidak ada gunanya sampai 0.2 selesai, karena SIP mengoptimalkan HashJoin yang saat ini tidak menghasilkan join yang benar.

**Prioritas baru: 0.1 → 0.2 → SIP (dulu P1) → sisanya seperti semula, digeser ke bawah.**

---

## 1. Kenapa Dokumen Lama Sudah Basi

`KONSOLIDASI_DOKUMEN.md` ditulis pada commit `1946b48` ("konsolidasi dokumen"). Sejak saat itu hingga HEAD (`56c80bb`):

- **35 file berubah, +6.181/-174 baris** di `kuzu-core/`
- **18 commit** landed, mayoritas langsung menutup gap yang didokumentasikan sebagai "KRITIS" atau "TIDAK ADA":

| Commit | Menutup gap |
|---|---|
| `a75e0dc` GDS Framework + Shortest Path | Fase 1+2 (GDS framework, 8 algoritma shortest path) |
| `7defc50` PhysicalRecursiveExtend | Path tracking untuk recursive extend |
| `3ccc1b4` + `6996865` SIP/SemiMask | Fase 3 (kerangka SIP — lihat catatan wiring di bawah) |
| `2b93624` nextval()/currval() | Fase 6.1 |
| `4a2a29e` SERIAL auto-increment | Fase 6.2 |
| `c9c22d3` array_value() | Fase 6.3 |
| `78ea6dc` Free Space Manager | Fase 5.1 (lihat catatan wiring di bawah) |
| `0afbafb` CREATE MACRO | Fase 6.5 |
| `4243ed5` Agg Key Dependency pass | Fase 4.4 |
| `ce233d6` Acc Hash Join Opt | Fase 4.2 |
| `12bba12` Correlated Subquery | Fase 4.1 |
| `3df8f74` Foreign Join PushDown | Fase 4.3 |

**Jangan pakai skor paritas "~82%" dari dokumen lama** — itu snapshot sebelum 18 commit ini. Status riil jauh lebih tinggi (lihat bagian 2).

---

## 2. Status Terverifikasi per Item (dari pembacaan kode, bukan dokumen)

### ✅ Nyata, Teruji, dan Tersambung ke Jalur Eksekusi

| Item | Bukti |
|---|---|
| GDS Frontier/BFS/Dijkstra framework | `kuzu-graph/src/gds/{frontier,compute,bfs_graph,output_writer,utils}.rs` — 2.547 baris, ada test |
| 8 algoritma shortest path (SSP/ASP/WSP/AWSP, destinations+paths) | `kuzu-algo/src/lib.rs:202-229` — terdaftar sebagai `TableFunction::CustomTable` dengan implementasi nyata, bukan placeholder. 6 test lulus. |
| PhysicalRecursiveExtend + path node/edge tracking | `kuzu-processor/src/physical_operator.rs:2601-2854` — rekonstruksi path penuh, semantik WALK/TRAIL/ACYCLIC |
| 17 optimizer pass (11 flat + 6 tree) — **melebihi C++ (16+)** | `kuzu-optimizer/src/passes.rs` — semua diverifikasi punya logika nyata (bukan stub), semua terdaftar di `optimizer.rs::Optimizer::new()` |
| nextval()/currval(), SERIAL, array_value(), CREATE MACRO | Diverifikasi ada di `kuzu-function/src/registry.rs`, `kuzu-catalog/src/lib.rs` dengan test |
| XOR operator, `RETURN *` | Bonus dari commit "akeh" (56c80bb) — tidak tercatat di dokumen manapun sebelumnya |

### ⚠️ Kerangka Nyata, TAPI Belum Tersambung ke Eksekusi Nyata (temuan baru sesi ini)

Ini bukan "belum diporting" — kodenya ADA dan teruji secara terisolasi, tapi **tidak pernah aktif saat query sungguhan dijalankan**. Ini beda kategori dari "hilang total" dan seharusnya jauh lebih murah untuk diselesaikan (tinggal menyambung, bukan membangun dari nol).

| Item | Yang sudah ada | Yang putus |
|---|---|---|
| **SIP/SemiMask** | `LogicalSemiMasker`, `PhysicalSemiMasker`, `NodeSemiMask`, `PhysicalScan::with_semi_mask()` — semua nyata & teruji (`kuzu-planner/src/logical_operator.rs:43-60`, `kuzu-processor/src/physical_operator.rs:21-124`) | `AccHashJoinOptimization` (`kuzu-optimizer/src/passes.rs:905-944`) **tidak pernah membuat** `LogicalSemiMasker` — hanya membungkus probe side dengan `Accumulate`. `processor.rs:302` **hardcode** `semi_mask: None` saat membangun `PhysicalHashJoin` dari rencana nyata. Verifikasi langsung: `grep -n semi_mask kuzu-processor/src/processor.rs` → baris 302 `semi_mask: None,` di jalur konstruksi asli (baris 1477+ yang `Some(mask)` hanya di kode test). |
| **Free Space Manager** | Implementasi buddy-system lengkap, `kuzu-storage/src/free_space_manager.rs` (315 baris) | Satu-satunya referensi di luar filenya sendiri: `pub mod free_space_manager;` di `lib.rs:14`. Tidak dipanggil `buffer_manager.rs` manapun. |
| **Zone Map Predicate** | `check_zone_map()`, `ColumnChunkStats` lengkap dengan test lulus, `kuzu-storage/src/predicate.rs` (203 baris) | Nol call-site di operator scan manapun (`PhysicalScan`, `column_chunk.rs`). Modul dikompilasi (`pub mod predicate;`) tapi tidak pernah dipanggil. |

### ❌ Masih Benar-Benar Hilang

| Item | Detail |
|---|---|
| Intersect enhancement | `PhysicalIntersect` (`kuzu-processor/src/physical_operator.rs:1434-1600`) masih `HashMap<u64, Vec<...>>` single-key. Belum reuse HashJoin shared-state, belum selection vectors, belum multi-column key. |
| ADBC extension | Nol referensi ADBC di seluruh workspace. Ada di LadybugDB, tidak ada di C++ Vela asli maupun Rust. |
| Cost-based join order (DP) | `join_order.rs` masih cardinality-aware **greedy**, belum dynamic-programming enumeration penuh seperti `CostModel`+`JoinTreeConstructor` C++. Untuk query umum sudah cukup baik; baru terasa di join kompleks 5+ tabel. |
| Weighted RecursiveExtend cost | Path tracking sudah ada, tapi cost masih depth-only (jumlah hop), belum lookup properti weight asli dari katalog untuk WSP/AWSP di jalur RecursiveExtend (algoritma shortest path standalone di `kuzu-graph` sendiri SUDAH punya Dijkstra berbobot — gap ini spesifik ke operator `RecursiveExtend` untuk pola MATCH variable-length). |
| 135 clippy warning | Naik dari 128 di dokumen lama — wajar mengingat volume fitur baru, tapi belum dibersihkan. |
| Correlated subquery unnesting di level *planner* (bukan optimizer) | Fungsional lewat `CorrelatedSubqueryUnnesting` optimizer pass, tapi C++ melakukannya di planner (`plan_subquery.cpp`). Beda arsitektur, bukan gap kapabilitas — **tidak direkomendasikan untuk dikerjakan** kecuali ada alasan konkret. |

---

## 3. Rencana Prioritas

Total estimasi sisa: **~19-30 hari kerja** (naik dari estimasi revisi 1 karena dua bug korektness fundamental di bagian 0 belum masuk hitungan sebelumnya — masih jauh di bawah estimasi dokumen lama 41-61 hari).

### 🔴🔴 Prioritas 0 — Perbaiki Property Access & HashJoin/CrossProduct (6-9 hari) — WAJIB SEBELUM SEMUA ITEM LAIN

Lihat detail lengkap di **Bagian 0** di atas. Ini bug korektness aktif, bukan optimasi — mempengaruhi hasil query yang salah secara diam-diam hari ini.

| # | Task | File Target | Hari |
|---|------|-------------|------|
| 0.1a | Perbaiki `evaluate_property_access` agar benar-benar resolve nama properti → indeks kolom fisik via skema tabel, bukan mengabaikannya | `kuzu-processor/src/expression_evaluator.rs:158-166` | 2-3 |
| 0.1b | Audit tempat lain yang mengasumsikan `PropertyAccess` bisa "collapse" ke variabel dasarnya (mis. `extract_variable_alias` di `kuzu-planner/src/join_order.rs` — ini OK untuk deteksi join condition, tapi pastikan tidak dipakai keliru di tempat lain) | Grep `PropertyAccess` di seluruh `kuzu-processor/`, `kuzu-planner/` | 1 |
| 0.1c | Test regresi luas: WHERE dengan properti non-PK (string & int), RETURN multi-properti, kombinasi keduanya | `kuzu-processor/tests/`, `kuzu-main/tests/` | 1 |
| 0.2a | ScanNode/ScanRel mengakumulasi (append) `intermediate_result`, bukan menimpa, saat mengarah ke HashJoin/CrossProduct | `kuzu-processor/src/processor.rs` (arm `ScanNode`/`ScanRel`) | 1 |
| 0.2b | HashJoin menurunkan `build_columns`/`probe_columns` sungguhan dari `join_keys` + skema tabel (butuh 0.1 selesai) | `kuzu-processor/src/processor.rs` (arm `HashJoin`), `physical_operator.rs:1609+` | 1-2 |
| 0.2c | Audit & perbaiki `PhysicalCrossProduct` dengan pola yang sama (`physical_operator.rs:1151-1165`) | `kuzu-processor/src/physical_operator.rs` | 0.5-1 |
| 0.2d | Test regresi: join ID tidak simetris (harus dapat hasil benar), join 0-match (harus 0 baris), join 3-tabel, cross product murni tanpa WHERE | `kuzu-main/tests/` | 1 |

### 🔴 Prioritas 1 — Nyalakan SIP/SemiMask (3-5 hari)

**Baru berguna setelah Prioritas 0 selesai** — SIP mengoptimalkan HashJoin; tidak ada gunanya mengoptimalkan operator yang belum menghasilkan join yang benar. Kerangkanya sudah 80% jadi, tinggal menyambung. Dampak: query multi-hop bisa 2-10x lebih cepat begitu aktif dan HashJoin sudah benar.

| # | Task | File Target | Hari |
|---|------|-------------|------|
| 1.1 | Buat pass baru (atau perluas `AccHashJoinOptimization`) yang benar-benar mengonstruksi `LogicalSemiMasker` di sisi scan node yang di-probe, ketika join selektif terdeteksi | `kuzu-optimizer/src/passes.rs` | 1.5 |
| 1.2 | Sambungkan hasil `LogicalSemiMasker` ke `PhysicalHashJoin`/`PhysicalScan` — ganti hardcode `semi_mask: None` di `processor.rs:302` dengan mask nyata dari rencana | `kuzu-processor/src/processor.rs` | 1.5 |
| 1.3 | Mekanisme berbagi mask antar-operator dalam pipeline (mis. `Arc<NodeSemiMask>` dibagi lintas tahap eksekusi, karena `processor.rs` mengeksekusi operator secara linear) | `kuzu-processor/src/processor.rs` | 1 |
| 1.4 | Test end-to-end: query multi-hop dengan filter selektif harus benar-benar memangkas scan (bukan cuma unit test terisolasi seperti sekarang) | `kuzu-main/tests/` | 1 |

### 🟡 Prioritas 2 — Sambungkan Free Space Manager & Zone Map (3-5 hari)

Murah karena implementasi sudah ada, ini murni integrasi.

| # | Task | File Target | Hari |
|---|------|-------------|------|
| 2.1 | Panggil `free_space_manager` dari jalur alokasi page di `buffer_manager.rs`/`storage_manager` — reuse ruang kosong alih-alih selalu growing file | `kuzu-storage/src/buffer_manager.rs` | 1.5 |
| 2.2 | Panggil `check_zone_map()` dari `PhysicalScan`/`column_chunk.rs` sebelum membaca chunk, skip jika predikat pasti tidak match | `kuzu-processor/src/physical_operator.rs`, `kuzu-storage/src/column_chunk.rs` | 2 |
| 2.3 | Test: buktikan chunk benar-benar di-skip (mis. lewat counter/instrumentasi), bukan cuma fungsi predicate-check yang lulus terisolasi | `kuzu-storage/src/predicate.rs` | 1 |

### 🟢 Prioritas 3 — Item Sedang (5-7 hari)

| # | Task | Referensi C++ | Hari |
|---|------|--------------|------|
| 3.1 | Weighted cost asli untuk `PhysicalRecursiveExtend` (bukan cuma depth) | `recursive_extend.h` + `weight_utils.h` | 1-2 |
| 3.2 | Intersect: reuse HashJoin shared-state, selection vectors, multi-column key | `intersect.cpp/h` | 1-2 |
| 3.3 | Cost-based join order enumeration (upgrade dari greedy) | `cost_model.cpp`, `join_tree_constructor.cpp` | 4-5 |

### ⚪ Prioritas 4 — Opsional / Housekeeping (3-5 hari)

| # | Task | Hari |
|---|------|------|
| 4.1 | Bersihkan 135 clippy warning | 2 |
| 4.2 | ADBC extension (kalau memang dibutuhkan — cek dulu apakah ada use case nyata, karena ini fitur LadybugDB-only, bukan di C++ Vela asli) | 3-5 |
| 4.3 | Setup CI/CD (belum ada per audit lama — cek apakah masih benar) | 1-2 |

---

## 4. Cara Pakai Rencana Ini

Untuk mengerjakan tiap item, pakai `/port-feature` (lihat `.github/prompts/port-feature.prompt.md`) dengan deskripsi fitur dari tabel di atas sebagai input. **Kerjakan Prioritas 0 dulu, tanpa kecuali** — ini bug korektness aktif yang mempengaruhi hasil query hari ini, bukan optimasi yang bisa ditunda. Prioritas 1 (SIP wiring) baru bernilai setelah Prioritas 0 selesai dan diverifikasi.

Sebelum mulai item apa pun, jalankan verifikasi cepat (`grep`/`git show` + trace eksekusi empiris via CLI, bukan hanya membaca kode statis) seperti yang dilakukan di sesi ini — jangan percaya status di dokumen manapun (termasuk dokumen ini) tanpa cross-check ke kode. Bug di Bagian 0 baru ditemukan justru karena verifikasi dilakukan lebih dalam dari sekadar membaca kode: butuh trace eksekusi nyata (`KUZU_DEBUG_PLAN`-style instrumentation sementara, dihapus setelah dipakai) untuk mengonfirmasi apa yang benar-benar terjadi saat query dijalankan, bukan apa yang tersirat dari struktur kode.
