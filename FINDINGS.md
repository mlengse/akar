# Akar — Findings

> **Fungsi dokumen:** jurnal temuan ber-tanggal (findings, incidents, audits,
> status verifikasi). Temuan yang sudah selesai dipindah ke `CHANGELOG.md`.
> Rencana kerja aktif ada di `implementation plan.md`. Bukan instruksi kerja.
>
> Asal temuan: audit Sulur ↔ Akar 2026-09-16 (daemon live `~/.sulur/engine/sulur.db`,
> biner `v0.2.1+11 (b0fbee1)`, DB 737 memori / 24.974 edge `Connected`).

Semua temuan audit sudah selesai: F1 `508328a`, F2 `ff1a995`,
F3 & F6 `2ba16d8`, F4 `ef792bb`, F5 `1270400` (riwayat di `CHANGELOG.md`).

---

## F13 — 2026-09-20: argumen agregat yang bukan kolom polos menghasilkan NULL (`SUM(expr)`) — TERBUKA

**Ranah:** akar (agregasi — resolusi argumen agregat). **Status:** TERBUKA — belum ada fix; ditemukan bersamaan F12 saat menulis tes regresi-nya.

### Gejala (tanpa error, hasil NULL)

`SUM(x)` hanya benar bila `x` adalah **kolom polos**; argumen terhitung menghasilkan `NULL`:

```cypher
MATCH (s:DT) RETURN SUM(s.bridges)                                          -- 18    (benar)
MATCH (s:DT) RETURN SUM(s.bridges * 2)                                      -- NULL
MATCH (s:DT) RETURN SUM(abs(s.bridges))                                     -- NULL
MATCH (s:DT) RETURN SUM(CASE WHEN s.phase='rem' THEN s.bridges ELSE 0 END)  -- NULL
MATCH (s:DT) RETURN COUNT(s.phase)                                          -- 2     (benar: COUNT hanya butuh kolom)
```

### Akar masalah

`resolve_agg_col_indices` (`akar-processor/src/physical/order_aggregate/aggregatehashtable.rs:613`) memetakan tiap argumen agregat ke **indeks kolom**: hanya `Variable`/`PropertyAccess` yang di-resolve, `Star` → `None` (sengaja, untuk `COUNT(*)`), dan **seluruh ekspresi lain jatuh ke `None`** lewat `_ => {}`. `None` dimaknai "tidak butuh kolom", sehingga agregat tidak menerima nilai apa pun dan hasilnya NULL.

### Bedanya dari F12

F12 mengembalikan **nilai salah yang non-null** (proyeksi ter-bind ke kolom lain). F13 mengembalikan **NULL** — "tidak didukung" yang tak terdokumentasi. Keduanya berasal dari pola yang sama (pemetaan ekspresi → indeks kolom dengan fallback diam-diam) tetapi di jalur kode berbeda (proyeksi vs agregasi).

### Dampak & mitigasi

- Dampak: agregat bersyarat (`SUM(CASE …)`) dan agregat atas ekspresi (`SUM(a*b)`, `SUM(abs(x))`) senyap menghasilkan NULL — metrik/laporan salah tanpa error.
- Mitigasi sementara: hitung ekspresi lebih dulu lalu agregasi atas kolom hasilnya (mis. lewat `WITH`) — **belum diverifikasi** apakah jalur `WITH` menghasilkannya dengan benar, jadi uji dulu sebelum dijadikan resep resmi; atau agregasi per-kondisi dengan `WHERE`.
- Perilaku saat ini **dipin** oleh `computed_aggregate_arguments_are_not_yet_supported` (`akar-main/tests/test_case_expression.rs`) agar batasannya terlihat dan setiap perubahan bersifat sengaja.

### Pendekatan perbaikan yang sudah dipetakan (belum dikerjakan)

Titik perbaikannya **bukan** di `aggregatehashtable.rs` (hot path, tidak memegang `FunctionRegistry`), melainkan di **mapper** `map_aggregate.rs::map_and_execute_aggregate`, yang justru memegang `ctx.function_registry`:

1. Sebelum membangun `agg_expressions`/`SharedAggregateState`, deteksi argumen agregat yang **bukan** `Variable`/`PropertyAccess`/`Star` (cermin `resolve_agg_col_indices`).
2. Untuk tiap argumen tersebut, evaluasi ekspresinya per-chunk dengan `ExpressionEvaluator` (pola yang sudah dipakai `map_projection.rs`), lalu **tambahkan hasilnya sebagai kolom trailing** pada chunk (`fields`/`field_types`/`field_names`).
3. Tulis ulang argumen itu menjadi referensi nama kolom sintetis, sehingga agregat fisik hanya melihat kolom polos dan **seluruh fast path tetap berlaku** (COUNT; Sum/Min/Max/Avg via `arrow_scalar_agg`).
4. Bila tidak ada argumen terhitung, lewati seluruh langkah → **no-op** untuk semua kueri yang ada (blast radius terbatas pada agregat ber-argumen ekspresi).

Yang **wajib** diverifikasi sebelum mengklaim selesai (inilah alasan perbaikan ini belum diambil sesi ini — berisiko memunculkan silent wrong result baru di jalur agregasi):

- `field_names` terisi pada input agregat (resolusi nama bergantung padanya; `SUM(s.bridges)` bekerja hari ini, jadi kemungkinan besar terisi — tetapi harus dibuktikan, bukan diasumsikan).
- Panjang array hasil `evaluate_arrow` vs `chunk.size` saat menambahkan kolom.
- Perilaku `sel_vector` (fast path skalar sudah bail-out saat `sel_vector.is_some()`, jadi agregat ber-`sel_vector` masuk jalur lain).
- Regresi: kueri agregat yang sudah benar (`SUM(kolom)`, `COUNT`, `GROUP BY`) harus menghasilkan nilai identik sebelum/sesudah.

### Langkah lanjut (usul)

1. Terapkan pendekatan mapper di atas.
2. Tes regresi untuk ketiga bentuk argumen terhitung; rewrite tes pin F13 (`computed_aggregate_arguments_are_not_yet_supported`).
3. Audit `AVG`/`MIN`/`MAX`/`STDDEV`/`VARIANCE`/`COLLECT` — semuanya memakai jalur resolusi yang sama.

---

## F12 — 2026-09-20: `CASE` di daftar proyeksi mengembalikan nilai kolom yang salah (silent wrong result) — RESOLVED (`P125`, gate 2,114)

**Ranah:** akar (proyeksi — pemetaan ekspresi ke kolom di `PhysicalProjection`). **Status:** SELESAI — akar masalah ditemukan, diperbaiki fail-safe, dan dipin oleh 7 tes regresi; ditemukan dari sisi Sulur (P1-OBS-1), tercatat juga di `sulur/docs/FINDINGS.md` #42.

### Gejala (akar 0.2.3, tanpa error)

`CASE` pada daftar `RETURN` mengembalikan nilai **kolom input lain**, bukan cabang `THEN`/`ELSE` —
tampak di-bind **berdasarkan posisi** ekspresi dalam daftar proyeksi alih-alih dievaluasi.

```cypher
CREATE NODE TABLE DT(id INT64, phase STRING, bridges INT64, PRIMARY KEY(id));
CREATE (:DT {id: 10, phase: 'rem', bridges: 7});
CREATE (:DT {id: 20, phase: 'supersedes', bridges: 11});

MATCH (s:DT) RETURN CASE WHEN s.phase = 'rem' THEN s.bridges ELSE 0 END AS a ORDER BY s.id;
-- aktual    : [[10], [20]]                       (s.id — bukan 7/0)
-- diharapkan: [[7], [0]]

MATCH (s:DT) RETURN s.id AS id, CASE WHEN s.bridges > 6 THEN 1 ELSE -1 END AS a ORDER BY s.id;
-- aktual    : [[10, 'rem'], [20, 'supersedes']]  (posisi ke-2 mengembalikan s.phase)
-- diharapkan: [[10, 1], [20, 1]]

MATCH (s:DT) RETURN SUM(CASE WHEN s.phase = 'rem' THEN s.bridges ELSE 0 END) AS t;
-- aktual    : [[None]]
-- diharapkan: [[7]]
```

### Bukan masalah `CASE`-umum — ekspresi lain benar

Diverifikasi pada build yang sama; semuanya mengembalikan nilai yang benar:
`RETURN s.bridges * 2`, `s.phase + '!'`, `COALESCE(s.bridges, 0)`, `abs(s.bridges)`.
Masalahnya spesifik pada ekspresi `CASE` — bentuk **searched** (`CASE WHEN … THEN …`) **dan** **simple**
(`CASE s.phase WHEN 'rem' THEN …`) sama-sama salah.

### Dampak & mitigasi sementara

- **Dampak:** agregat bersyarat gaya SQL yang lazim ditulis `SUM(CASE WHEN … THEN … ELSE … END)` mengembalikan hasil salah **tanpa error** — menyesatkan metrik/laporan. Sulur harus mengubah agregat metrik dream per-fase menjadi bentuk terfilter karena ini.
- **Mitigasi di sisi pemakai:** tulis agregat per-kondisi sebagai `MATCH … WHERE <kondisi> RETURN SUM(x)` (terverifikasi benar) alih-alih `SUM(CASE …)`.
- Belum ada tes pin di akar untuk repro di atas.

### Penutupan (2026-09-20, P125 — gate 2,114)

Akar masalah bukan di binder/planner (binder `Expression::Case` dan `ExpressionEvaluator::evaluate_case` sudah benar) melainkan di **pemilihan jalur proyeksi**: `projection_needs_expression_eval` (`akar-processor/src/processor/mapper/map_projection.rs`) menyebut varian yang *komputasional* dan **melewatkan `Expression::Case`**, sehingga proyeksi CASE mengambil jalur "kolom biasa"; karena `resolve_projection_column_expand` hanya me-resolve `Variable`/`PropertyAccess`, hasilnya `None` → pemanggil jatuh ke fallback posisional `column_indices = (0..expressions.len())`.

Perbaikan: predikat dibalik menjadi **fail-safe** — sebut varian yang **dapat di-resolve sebagai kolom** (`Variable`/`PropertyAccess`/`Star`), kirim semua ekspresi lain ke evaluator per-baris. Ini menutup seluruh kelas bug (varian ekspresi baru otomatis dievaluasi), bukan hanya `Case`. `Star` tetap di set "kolom" karena fallback posisional = "salin semua kolom" adalah perilaku yang benar bila Star bocor dari binder.

Tes `akar-main/tests/test_case_expression.rs` (7): searched CASE, simple form, CASE di posisi proyeksi kedua (repro orisinal), CASE di `WHERE`, CASE bercabang string + alias, penjaga "ekspresi lain tidak berubah", dan pin batasan F13.

**Catatan penting:** `SUM(CASE …)` pada repro awal **tidak** ikut tertutup — itu jalur agregasi yang berbeda dan dicatat sebagai **F13** di atas (mengembalikan NULL, bukan nilai salah).

---

## F11 — 2026-09-19: Evolusi Arsitektur Sulur ke Rust & Pensiun Bertahap akar-server — RENCANA

**Ranah:** akar (arsitektur akar-main, akar-server, batas domain §13).
**Status:** RENCANA — keputusan strategis pasca audit `ai-memory` dan standarisasi `hermes-plugins`.
**Konteks:**
1. **Penyelesaian Paradoks ADR-02:** Selama ini `akar-server` dipertahankan di workspace Akar sebagai kompromi (ADR-02) karena Sulur ditulis dalam Python dan memerlukan TCP broker tunggal untuk menghindari bentrok lock file database. Padahal, SPEC.md §13 menyatakan prinsip dasar: *"Akar ships no server — embedded library only"*.
2. **Dampak Migrasi Sulur ke Rust:** Begitu Sulur dimigrasikan menjadi binary Rust mandiri (`sulur-server` yang meng-embed `akar-main`), Sulur akan langsung mengontrol file lock `sulur.db` secara in-process. 
3. **Pensiun `akar-server` dari Jalur Produksi:**
   - Crate `akar-server` tidak lagi dibutuhkan oleh Sulur dalam lingkungan produksi.
   - Seluruh overhead TCP JSON loopback, serialisasi wire, dan isu locking socket tereliminasi.
   - Akar kembali 100% menjadi *pure embedded library* tanpa kontradiksi dokumentasi. Crate `akar-server` diturunkan statusnya menjadi *test harness / optional wire reference* saja.
4. **Kesiapan `akar-main` untuk Direct Embedding:**
   - Diperlukan audit pada `akar-main` terkait kemudahan multi-threaded access dan pembagian koneksi via `Arc<Database>` pada async runtime Tokio tingkat tinggi yang akan dipakai oleh Sulur Rust.

---

## F10 — 2026-09-19: Komparasi Arsitektur dengan ai-memory & Adopsi Primitif Komputasi Memori — RENCANA

**Ranah:** akar (akar-function, akar-dream, akar-search, akar-vector, akar-extension).
**Status:** RENCANA — referensi arsitektur dari audit proyek `ai-memory` (Fabio Akita / MIT).
**Konteks:** Perbandingan antara engine basis data grafik Akar dengan sistem memori agent `ai-memory`.
Batas domain (§13) tetap ditegakkan: Akar tetap embedded database library murni (tanpa server daemon atau hooks agent).
Namun, terdapat 5 primitif komputasi & retrieval dari `ai-memory` yang bernilai tinggi untuk diadopsi langsung ke Akar:

1. **Formula Decay Ebbinghaus (Retensi Temporal Berbasis Akses):**
   `ai-memory` menggunakan rumus retensi matematis murni:
   $$R = \text{salience} \cdot e^{-\lambda \Delta t} + \sigma \cdot \ln(1 + \text{access\_count}) \cdot e^{-\mu \cdot \Delta t_{\text{last}}} \cdot (1 + w_{\text{breadth}} \cdot \ln(\text{actors}))$$
   Di Akar, fase NREM `akar-dream` saat ini menggunakan pelemahan statis konstan (`weaken_edge(..., 0.05)`). Diperlukan scalar function Cypher di `akar-function` dan integrasi formula eksponensial ini ke NREM prune/strengthen cycle.
2. **In-Process Pure-Rust Local Embeddings via Candle (`akar-vector`):**
   `ai-memory` mengintegrasikan `candle-core` dan `tokenizers` untuk komputasi embedding teks lokal tanpa ketergantungan API eksternal. Di Akar, `akar-vector` saat ini hanya mengelola index HNSW dan `akar-llm` bergantung pada HTTP REST. Fitur opsional `candle` di `akar-vector` akan membuat Akar 100% mandiri dan offline-first.
3. **Multi-Stream Hierarchical RRF Fusion (L0 Abstract vs L1 Body) (`akar-search`):**
   `ai-memory` membedakan embedding ringkasan/abstrak (L0) dari embedding konten penuh (L1) dalam 5-way RRF. Akar dapat memperluas `PhysicalHybridScan` dan `akar-optimizer` untuk multi-property vector scan berbobot hierarkis bersama BM25 Tantivy.
4. **Authority / Metadata-Aware Rank Multiplier (`akar-processor`):**
   Penyesuaian skor pasca-fusi berbasis otoritas kategori/label (misal: node bertipe `:Decision` atau `:Rule` mendapat multiplier lebih tinggi dibanding log episodik) secara tervektorisasi pada Arrow chunk sebelum `LIMIT`.
5. **Ekstensi `akar-markdown` / Open Knowledge Format (OKF) Reader (`akar-extension`):**
   Ekstensi table function `CALL read_markdown_wiki('/path')` untuk memetakan direktori Markdown + YAML frontmatter + `[[wikilinks]]` menjadi Node Table (`Page`) dan Relationship Table (`LINKS_TO`) secara langsung di Cypher.

---

## F9 — 2026-09-19: agenda lanjut terpilih (investigasi F7 + hardening kematian senyap daemon) — RENCANA

**Ranah:** akar (storage/WAL + daemon lifecycle). **Status:** SELESAI — kedua item dikerjakan di P114 (`cd31526`);
tercatat sebagai urutan kerja berikutnya setelah verifikasi live DAE (F8) selesai.
**Sumber keputusan:** sesi 2026-09-19 — pilihan #2 dan #3 dari daftar kelanjutan pasca-verifikasi DAE.

### Item 1 — Investigasi F7 (`set.rs`, jalur replay edge)

- Repro mandiri: build lokal pada copy direktori DB live + `dataset/wal-corrupt-edge-index-20260918/wal.log`
  (fixture sendirian bukan repro mandiri — perlu salinan DB-nya).
- Inspeksi `akar_processor/physical/write_ops/set.rs` — invariant page edge. Tujuan: tambah guard
  saat **menulis** record update edge (bukan hanya saat replay) supaya WAL tidak bisa lahir dalam
  keadaan tak-replayable oleh penulisnya sendiri.
- Pertimbangkan mode `--salvage`/`--skip-wal` resmi (saat ini `mv wal.log` = prosedur operator,
  bukan produk) + log eksplisit "WAL diabaikan, N transaksi belum di-checkpoint hilang".
- Status rujukan: **F7 (TERBUKA)** — satu-satunya temuan akar yang masih terbuka.

### Item 2 — Hardening kematian senyap daemon (F7 sampingan #1)

Keputusan belum diambil. Opsi: panic-hook ke daemon log / tangkap abort karena OOM / log
`memory allocation … failed` (saat ini `sulur.db.err.log` 0 byte padahal 4 spawn mati tanpa
jejak 23:56–00:00). Target: proses berhenti tanpa jejak harus bisa dilacak alasan berhentinya.
Klaim F3/F6 "daemon mati tiap ±6 menit sudah tertutup" **belum terverifikasi live** — verifikasi
ulang setelah hardening terpasang.

---

## F8 — 2026-09-19: verifikasi live DAE (fix #38 Sulur) terhadap daemon produksi — TERVERIFIKASI

**Ranah:** akar (daemon/DB produksi) ↔ sulur. **Status:** TERVERIFIKASI — DAE pass penuh + resume
berjalan tanpa kerusakan, schema DDL tanpa `DEFAULT` diterima.
**Biner:** `~/.cargo/bin/akar_server.exe` v0.2.3 (dibangun 2026-09-18 23:34, HEAD `40415b9`) — sama dengan
yang dibuktikan sehat di F7 (copy DB +`mv wal.log` → listen).
**DB:** `~/.sulur/engine/sulur.db` (daemon live, pid 3268 / port 9876, sidecar token OK).
**Sumber:** verifikasi sisi kliem (repo Sulur, fix #38 commit `2464f40`) — replay DDL drive lewat
`DaemonClientStore` dari repo, bukan biner akar.

### Hasil verifikasi (live, 912 memori / max id 929)

1. **Schema DDL diterima tanpa `DEFAULT`:** keenam kolom `protected`, `dae_self_weight`,
   `dae_neighbour_k`, `dae_schema_version`, `dae_computed_at` (+ `dae_embedding`) ter-declare dan
   terbaca oleh daemon (`MATCH … RETURN m.<kolom>` → `None` sebelum pass). F2 (`ef792bb`, "ALTER ADD
   tanpa `IF NOT EXISTS`") memang sudah tertunda; di sini **`ADD kolom` polos (tanpa `DEFAULT`) terbukti
   tidak ditolak daemon** — `DEFAULT` yang digugurkan fix #38 bukan syarat agar DDL masuk.
2. **Pass penuh:** `SULUR_DAE_RESUME=0` → `computed=894, total=894, resumed_from=None`,
   `elapsed≈6,5 s`, `dim=384`, `batch_size=100`. 18 memori dieksklusi karena `embedding IS NULL`
   → `computed` ekspektasi 894, **bukan** 912. Konsistensi ini tidak boleh dianggap kerusakan PK.
3. **Pass resume:** pass berikut tanpa env → `computed=0, resumed_from=929`, `elapsed≈0,3 s` —
   watermark `Meta.dae_checkpoint_id=929` dihormati, tidak ada double compute.
4. **PK utuh:** `m.id` tetap ber-tipe `int` (spot: id 1, 400, 929), `count(Memory)=912` sebelum &
   sesudah semua pass. Tidak ada korupsi index/halaman (tidak ada `Edge index` panic di daemon log).

### Temuan sampingan (perlu dicatat)

1. **Penulis konkuren di DB yang sama:** saat sesi verifikasi, `dae_checkpoint_id` sudah berisi `929`
   padahal skan Meta awal hanya memuat `afe_processed_ids` → **satu engine Sulur lain di host yang sama
   menyelesaikan full DAE pass secara bersamaan** selama jeda verifikasi. Daemon sendiri bukan penulis
   Meta; yang menulis adalah klien. Untuk verifikasi lanjutan di DB live, jangan asumsikan state diam.
2. **Tulis batch besar tidak rentan di jalur DAE:** satu pass menulis 894 baris (setiap baris
   `SET … dae_*`) tanpa satu pun `daemon not answering` — kontras dengan temuan F7 no.2 (batch `sulur_recount write`
   28 entri gagal); kemungkinan perbedaan di jalur tulis/retry engine, layak ditelusuri terpisah.
3. **`protected` tetap `NULL`:** DAE tidak menyentuh kolom `protected` (semua 0/NULL setelah pass) —
   konsisten; `set_protected` adalah satu-satunya penulis.

### Catatan verifikasi

- Konfirmasi sisi akar yang dibutuhkan F7 (jalur replay edge `SET`) **tidak** dijalankan di sini —
  sesi ini hanya memvalidasi daemon tidak rusak saat klien menulis DAE volume penuh + resume.
- DAE pass penuh tidak pernah memicu replay WAL: pass berjalan normal, daemon tetap hidup.

---

## F7 — 2026-09-18/19: replay WAL gagal di jalur **edge update** (`Edge index 0 out of range`) — TERATASI (`cd31526`)

**Ranah:** akar (storage / WAL replay). **Status:** TERATASI di tingkat kode — guard tulis (P114.1) mencegah WAL lahir tak-replayable dan mode salvage resmi (P114.2) menyediakan jalur pemulihan; verifikasi live ulang pada daemon produksi belum dijalankan.
**Biner:** `~/.cargo/bin/akar_server.exe`, dibangun ulang **2026-09-18 23:34** (tree = `v0.2.3`, HEAD `40415b9`).
**DB:** `~/.sulur/engine/sulur.db` (daemon Sulur/Hermes, live).
**Konteks:** batch harian cron `belajar-puskesmas-notebooklm` (06:00) — daemon sudah mati sebelum job jalan.

### Gejala

Setiap spawn daemon gagal total (bukan hanya ping gagal):

```
Failed to open database at 'C:\Users\puske\.sulur\engine\sulur.db':
  WAL recovery failed (database may need manual repair):
  WAL recovery edge update failed: page: Edge index 0 out of range.
  Refusing to start with an empty database — check the WAL.
```

`wal.log` saat itu **1.211 byte** (mtime 2026-09-18 23:53), tulis terakhir yang tercatat di
`sulur.db.daemon.log`: `SET: updated 30 rows in 'Memory'` + `SET: updated 4 rows in 'Connected'`
(23:53:00). Checkpoint kolom (`col_13_*.meta`) juga 23:53 → seluruh data s/d checkpoint utuh di mirror.

### Bukan varian F5

P2-WAL-1 (`1270400`, "replay duplicate-PK insert last-write-wins") menutup jalur replay **INSERT**.
Kegagalan ini di jalur replay **edge UPDATE** (`SET … Connected`) — invariant `Edge index 0 out of range`
pada halaman edge belum tertangani. Kandidat kuat: record update edge yang ditulis build ini tidak
replayable oleh build yang sama (daemon yang menulis WAL = biner 23:34 itu sendiri, hidup 23:50:33–23:53+).

### Bukti: WAL adalah satu-satunya penghalang

Copy direktori DB → `mv wal.log` keluar dari copy → `akar_server --db <copy> --port 59998` langsung
`Akar server listening` (restore 5 tabel). Jadi biner + DB sehat; hanya replay WAL yang gagal.
Fixture byte-identik: `dataset/wal-corrupt-edge-index-20260918/wal.log` (magic `AKAR` v2, 1.211 B).
Repro butuh salinan direktori DB live + fixture ini di dalamnya — fixture sendirian bukan repro mandiri.

### Pemulihan yang dipakai (terverifikasi)

```bash
R="C:/Users/puske/AppData/Local/Temp/sulur-recovery-20260918"
cp ~/.sulur/engine/sulur.db/wal.log "$R/wal.log.corrupt-backup"   # evidence dulu
mv ~/.sulur/engine/sulur.db/wal.log "$R/wal.log.corrupt-20260918T2358"   # KELUAR dari dir DB
PYTHONPATH= .venv/Scripts/python.exe tools/stop_rival_daemons.py --db "<db>"
PYTHONPATH= .venv/Scripts/python.exe tools/sulur_daemon_ctl.py --db "<db>" fix
```

**Biaya data: 0 memori hilang** — checkpoint 23:53 sudah memuat semua (808 memori sebelum & sesudah;
870 setelah batch 29 entri malam itu ditulis ulang lewat daemon).
`daemon_ctl fix` sempat mencetak `did not become healthy in time` padahal DB-open-nya memang lambat;
keberhasilan harus dikonfirmasi dari baris `Akar server listening` + `status`.

### Temuan sampingan (perlu diverifikasi terpisah)

1. **Kematian daemon senyap masih terjadi.** 4 spawn dalam ~25 menit: pid 19832 (23:56:18),
   13292 (23:57:52), 14320 (23:58:57), 14724 (00:00:49). Setelah pid 14320 berhenti: **tidak ada**
   panic/abort/`memory allocation … failed` di `sulur.db.daemon.log`, `sulur.db.err.log` 0 byte,
   `tasklist` kosong → proses berhenti tanpa jejak (kill eksternal, atau abort sebelum flush log).
   F3/F6 sudah diklaim tertutup (`2ba16d8`); di host ini kematian tidak-terjadwal masih teramati,
   jadi klaim "daemon mati tiap ±6 menit sudah tertutup" **belum terverifikasi live**.
2. **Batch tulis besar rentan:** satu proses `sulur_recount.py write` untuk 28 entri gagal total
   (`daemon not answering … pid 13292`); dipecah 4×7 entri dengan `fix` sebelum tiap chunk →
   `written=7 failed=0` empat kali, <2 s per chunk. Saat daemon hidup-pendek, tulis kecil + `fix` per chunk.
3. **Doc hygiene:** header dokumen ini merujuk `implementation plan.md` yang tidak ada di repo
   (tidak ada `PLAN.md` di root akar).

### Langkah lanjut (usul, belum dikerjakan)

- Reproduce dengan fixture: jalankan build lokal pada copy DB + `dataset/wal-corrupt-edge-index-20260918/wal.log`,
  lihat `akar_processor/physical/write_ops/set.rs` (jalur update edge) — invariant page edge.
- Tambah guard saat *menulis* record update edge (bukan hanya saat replay), supaya WAL tidak bisa
  lahir dalam keadaan tak-replayable oleh penulisnya sendiri.
- Tambah mode `--salvage`/`--skip-wal` resmi di akar-server (pindah-manual `wal.log` = prosedur operator,
  bukan produk) + log eksplisit "WAL diabaikan, N transaksi belum di-checkpoint hilang".

### Penutupan (2026-09-19, `cd31526` — P114)

Ketiga langkah di atas dikerjakan:

1. **P114.1 — guard tulis.** `PhysicalSet` tidak lagi menulis record update edge untuk indeks di luar
   rentang maupun edge yang sudah di-tombstone; akar F7 (rel scan tanpa kolom `_id` → nilai properti
   dibaca sebagai indeks edge) tertutup di titik lahirnya record. Tes `set_edge_guard_*` (akar-processor).
2. **P114.2 — salvage mode.** `SystemConfig::skip_wal` (default `false`) → `StorageManager::set_skip_wal`;
   `recover()` mencatat + melewati record yang gagal alih-alih membatalkan open. Opsi operator manual
   `mv wal.log` kini fitur produk: `akar-server --skip-wal` · `akar-cli [db] --skip-wal|--salvage`.
   Strict tetap default (P61.3). Tes `test_wal_recovery_salvage_mode_skips_unplayable_record`.
3. **P114.3 — log hardening** (F9 item 2). `akar_server::daemon_log`: panic hook + jejak kegagalan
   alokasi (OOM) + marker `START`/`EXIT` bertimestamp+pid, semuanya di-flush ke stderr sebelum handler
   default berjalan. Tes `now_ms_is_positive`, `logging_allocator_delegates_to_system`.

Gate `test [akar-core]`: **2,107 passed / 0 failed / 0 ignored** (2,102 → 2,107). Verifikasi live
(memakai `dataset/wal-corrupt-edge-index-20260918/wal.log` pada salinan direktori DB produksi) masih
perlu dijalankan agar F7 dapat ditutup penuh; `--skip-wal` kini jalur resmi untuk keperluan itu.
