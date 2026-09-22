# Akar — Findings

> **Fungsi dokumen:** jurnal temuan ber-tanggal (findings, incidents, audits,
> status verifikasi). Temuan yang sudah selesai dipindah ke `CHANGELOG.md`.
> Rencana kerja aktif ada di `implementation plan.md`. Bukan instruksi kerja.
>
> Asal temuan: audit Sulur ↔ Akar 2026-09-16 (daemon live `~/.sulur/engine/sulur.db`,
> biner `v0.2.1+11 (b0fbee1)`, DB 737 memori / 24.974 edge `Connected`).

Semua temuan audit sudah selesai: F1 `508328a`, F2 `ff1a995`,
F3 & F6 `2ba16d8`, F4 `ef792bb`, F5 `1270400` (riwayat di `CHANGELOG.md`).
F9 & F7-reguard `cd31526` (P114), F12 `P125`, F13 `P126`, F11 (pensiun `akar-server`: P127 +
P124.1–P124.2 di akar, P4-RETIRE-1/P4-BOUND-1 di sulur), dan F10 (seluruh 5 primitif
yang diadopsi: P119 decay Ebbinghaus, P120 embedding lokal, P121 hierarchical RRF +
authority multiplier, P122 `akar-markdown`) — semuanya sudah ditutup dan dipindah ke
`CHANGELOG.md`; hanya temuan yang masih terbuka/masih punya celah yang tinggal di berkas ini.

---

## F15 — 2026-09-21: `SystemConfig::default()` memakai `checkpoint_threshold = -1` (checkpoint tiap tulisan) — TERBUKA

**Ranah:** akar (`SystemConfig` default) ↔ sulur (jalur embedded Python + `sulur-server`).
**Status:** TERBUKA — punya item **`implementation plan.md` Iterasi 6 / P128**; terungkap saat Iterasi 4
(P4-RETIRE-1) dan dihindari (bukan diperbaiki) dengan mengirim threshold eksplisit.
Investigasi **P128.1 sudah selesai** (hasilnya di bawah); keputusan arah **P128.2 belum diambil**.

### Gejala

`SystemConfig::default().checkpoint_threshold == -1`, artinya **setiap penulisan langsung memicu
checkpoint**. Bentuk pembukaan database yang paling natural bagi pemakai baru —
`Database::new(path, SystemConfig::default())` — karena itu menghasilkan perilaku tulis kelas
"satu checkpoint per tulis" tanpa satu pun error atau peringatan.

### Bukti

1. **Sulur Rust (ditemukan di Iterasi 4).** `sulur/crates/sulur-server` awalnya membuka database dengan
   `SystemConfig::default()`. `Server::bind` kini mengirim `ServerConfig::checkpoint_threshold`
   (default 16 MiB, override `SULUR_CHECKPOINT_THRESHOLD`, forward dari `sulur_daemon_ctl`) secara
   eksplisit — tanpa itu daemon Rust **benar tetapi jauh lebih lambat** daripada binary yang
   digantikannya. Ini perbedaan performa terbesar yang ditemukan di iterasi tersebut, dan sebabnya
   satu baris default, bukan algoritma.
2. **Sulur Python (jalur embedded).** Gate `perf: store 100 memories < 2s`
   (`sulur/tests/test_suite.py:513`) mendarat **~20–25 s** untuk 100 store; komentar tesnya sendiri
   (baris 524–528) menyebut sebabnya: *"embedded default checkpoint_threshold=-1"*, dan mencatat bahwa
   jalur produksi (daemon, threshold 16 MiB) tidak terdampak. Ambang tes dinaikkan 2 s → 30 s karena itu.
   Gate yang sama sudah dua kali jatuh: **42,71 s** (2026-09-17) dan **39,15 s** (2026-09-21) —
   `sulur/docs/FINDINGS.md` #37 dan #44.
3. **Nilai threshold-nya sendiri sudah diputuskan sebelumnya.** P68 memisahkan checkpoint dari jalur
   tulis dan menetapkan 16 MiB di jalur daemon, jadi angka yang benar sudah diketahui di repo ini.
   Yang belum selesai adalah **default** di `SystemConfig`.

### Dampak

Trap senyap. Sebuah API default seharusnya menjadi pilihan yang wajar; di sini ia menghasilkan
performa yang jauh lebih buruk tanpa sinyal apa pun kepada pemanggil. Semua konsumen embedded
(Sulur Python, tooling, tes, dan siapa pun yang menulis `SystemConfig::default()` di masa depan)
mewarisi biaya itu, dan biayanya terlihat seperti "Sulur/Akar lambat", bukan seperti "default-nya salah".

### Hasil investigasi P128.1 (2026-09-23) — apa yang sebenarnya dijamin `-1`

Diperiksa: `SystemConfig::default()` (`akar-main/src/database.rs`), `maybe_auto_checkpoint`
(`akar-main/src/connection/query.rs`), `maybe_checkpoint` / `commit_transaction` / `recover()`
(`akar-storage/src/lib.rs`), `checkpoint()` (`akar-storage/src/checkpoint.rs`), `flush_to_disk`
(`akar-storage/src/wal.rs`), plus P60.1/P60.2/P68/P114 di `CHANGELOG.md`.

**Durabilitas tidak berasal dari checkpoint.** `commit_transaction` Step 1 menulis record `Commit`
lalu **fsync** — inline (`wal.flush_to_disk()` → `file.sync_data()`) atau lewat group commit
(`GroupCommit::flush()` menunggu fsync yang dimulai setelah enqueue). Doc-comment-nya menyatakan hal
yang sama untuk **setiap** mode threshold: *"Since P60.2 the SQL write path emits typed
`Insert/Delete/Update WAL records, so committed data is durable from the WAL alone"*. `recover()`
memang persis begitu: Phase 1 muat mirror kolom, Phase 2 `wal.load_from_disk()` + replay record data
bertipe, baru mirror ditulis ulang. Checkpoint hanya memutuskan **kapan** mirror kolom ditulis ulang
dan **kapan** WAL dipotong.

**Asal-usul `-1` sudah kedaluwarsa.** P60.1 menemukan bahwa klaim "WAL fsync sudah menjamin
durabilitas" **keliru untuk jalur SQL** waktu itu: WAL hanya membawa marker `Commit` + blob
`LocalWALData` kosong yang di-skip replay, sehingga **mirror kolom adalah satu-satunya sumber
recovery** dan `-1` (selalu checkpoint) yang membuat tiap tulisan durable. P60.2 (`d0a1447`,
2026-08-24) menutup itu dengan typed Insert/Delete/Update WAL records. Jadi `-1` adalah **default
yang tertinggal dari pengaturan pra-P60.2** dan belum pernah ditinjau ulang — bukan pilihan
durabilitas yang diputuskan sadar.

**Yang benar-benar berubah bila ambangnya angka byte** (bukan durabilitas):
1. **Jendela replay saat crash.** `-1` menyisakan WAL nyaris kosong; `N` byte berarti sampai N byte
   harus di-replay. Ini satu-satunya argumen pro-`-1` yang jujur: bila ada record tak-replayable
   (kelas F7), blast radius-nya lebih kecil. Sejak P114.1 penulis tak bisa lagi melahirkan record
   seperti itu, tetapi verifikasi live F7 masih terbuka.
2. **Jejak disk `wal.log`** — dibatasi N byte (dan karena itu pula waktu replay).
3. **Biaya per tulis** — inilah yang mahal: `-1` memicu, untuk **setiap** tulis, persist ulang
   seluruh mirror kolom + `BufferManager::flush_all()` + `wal.clear()` + marker + fsync. (P60.1
   sudah memangkas separuh: Step 2 di-skip ketika checkpoint pasti jalan.) Angka Sulur ~20–25 s per
   100 store embedded berasal dari sini, bukan dari WAL.

**Kesimpulan investigasi:** tidak ada jaminan durabilitas yang hilang bila default berpindah ke ambang
byte; yang berubah hanyalah panjang jendela replay dan biaya per tulis. Karena itu pertanyaan lama
"`-1` mungkin memang disengaja" **sudah terjawab: tidak.** Yang tersisa murni trade-off (P128.2).

### Temuan lanjutan (audit konsumen + pra-ukur, 2026-09-23)

Empat hal yang mengubah bentuk keputusan, ditemukan setelah P128.1:

1. **`akar-python` tidak punya permukaan `SystemConfig` sama sekali.** `Database` Python dibuka dengan
   `akar_main::Database::new(path, Default::default())` (`akar-core/akar-python/src/lib.rs:57`) — tanpa
   argumen config apa pun. Artinya opsi "Sulur mengirim ambangnya sendiri dari jalur embedded Python"
   **tidak mungkin hari ini**: ia menuntut API baru di akar-python lebih dulu. Jadi arah (B) bukan
   "cukup dokumentasi", melainkan "dokumentasi **+ API config di akar-python**".
2. **Jalur daemon sudah memilih 16 MiB, dan menyebut `-1` sebagai pemulihan perilaku lama.**
   `akar-server --checkpoint-threshold` default `16 * 1024 * 1024` (`akar-server/src/bin/akar_server.rs:87`),
   dengan help text *"disable auto-checkpoint, or -1 to restore checkpoint-per-write."* Repo ini karena
   itu sudah memutuskan 16 MiB sebagai default produksi dan `-1` sebagai opt-in legacy; yang tersisa bagi
   P128.2 adalah menyelaraskan default **pustaka** dengan keputusan itu.
3. **`-1` sudah dua kali menimbulkan cacat, bukan hanya lambat.** P67 (`0.1.14`): dengan `-1` setiap commit
   memicu auto-checkpoint, dan drain 30 s di jalur itu **selalu timeout** di bawah penulis konkuren
   (Hermes gateway + klien kairos) — diperbaiki dengan melewati drain untuk auto-checkpoint. Ditambah
   amplifikasi tulis di bawah, `-1` punya rekam jejak dua kelas masalah.
4. **Blast radius perubahan default (bahan P128.3).** `SystemConfig::default()` dipakai oleh default FFI
   `akar-c`, `akar-cli` (mewarisi `-1`), `akar-python` (mewarisi), `akar-main/src/bin/ladybug.rs`, banyak
   bench, dan helper tes (`test_helpers::setup_db`/`setup_db_on_disk`) plus beberapa tes crash-recovery.
   Dua konsekuensi yang perlu disebut terang-terangan: (a) `storage_io_bench::bench_insert_throughput`
   memakai default, jadi **benchmark throughput akar sendiri mengukur jalur `-1`** — kalau angka
   `SPEC.md` §12.1 berasal dari sana, ia perlu diukur ulang setelah perubahan; (b) suite crash-recovery
   ditulis di atas semantik default lama, jadi (A) bukan perubahan satu baris.

**Pra-ukur P128.4 (debug, 100 `CREATE` ke satu tabel 2 kolom, satu mesin, 2026-09-23):**

| `checkpoint_threshold` | 100 creates |
|---|---|
| `-1` (checkpoint tiap tulis) | **2,648 s** |
| 16 MiB | **0,474 s** |
| `0` (tidak pernah — referensi batas bawah, bukan kandidat) | 0,470 s |

`-1` **5,6×** lebih lambat, dan 16 MiB tak terbedakan dari "tidak pernah checkpoint" pada volume ini
(WAL tak pernah mencapai 16 MiB). Angka absolut Sulur (~20–25 s per 100 store) lebih besar karena setiap
memori menulis vektor 384-dim, sehingga mirror yang ditulis ulang tiap checkpoint jauh lebih berat —
arah dan sebabnya sama. Probe-nya sementara dan tidak ikut di-commit.

**Catatan dokumentasi:** `SPEC.md` tidak menyebut `checkpoint_threshold` maupun auto-checkpoint sama
sekali, jadi apa pun arahnya, kebijakan checkpoint perlu satu paragraf di SPEC (§15 atau §17).

### Langkah lanjut (usul, belum dikerjakan)

- **Putuskan salah satu:** ubah default `SystemConfig` menjadi threshold nyata (mis. 16 MiB, selaras
  jalur daemon P68), **atau** pertahankan `-1` sebagai pilihan sadar dan dokumentasikan terang-terangan
  di doc-comment `SystemConfig` beserta alasan durabilitasnya.
- ~~Sebelum mengubah default, periksa kontrak durabilitasnya.~~ **Sudah dijawab oleh P128.1 di atas:**
  `-1` bukan pilihan durabilitas yang disengaja, melainkan default yang tertinggal dari pengaturan
  pra-P60.2. Yang tersisa dari kekhawatiran ini hanyalah satu argumen jujur (jendela replay), dan itu
  masuk keputusan P128.2 — bukan alasan untuk menahan default apa adanya.
- Setelah arahnya jelas, pertimbangkan `checkpoint_threshold` eksplisit di jalur **embedded Python**
  Sulur juga (saat ini hanya jalur daemon yang mengirimnya), sehingga flake #37/#44 dihapus pada sebabnya
  alih-alih dengan menaikkan ambang tes.
- Verifikasi harus memakai pengukuran yang sadar beban: satu run terisolasi per hipotesis, bukan suite
  berulang di bawah beban paralel (lihat catatan prosedur di `sulur/docs/FINDINGS.md` #44).

---

## F14 — 2026-09-20: penulisan tepi tidak bisa di-batch dari parameter `UNWIND` (`Variable 'r' not found`) — TERBUKA

**Ranah:** akar (binder/planner — resolusi variabel `UNWIND` dari dalam pola `MATCH`).
**Status:** TERBUKA — terungkap saat P123.2; dihindari (bukan diperbaiki) dengan jatuh ke satu pernyataan per tepi.
**Konteks:** `akar-main/src/bulk.rs` ingin menulis batch tepi lewat satu
`UNWIND $rows AS r MATCH … CREATE/MERGE …` seperti jalur node.

### Gejala

Setiap bentuk yang seharusnya bisa, gagal atau diam-diam tidak match:

```
UNWIND $rows AS r MATCH (a:M {id: r.source}), (b:M {id: r.target}) MERGE …
  -> Execute error: Variable 'r' not found in chunk field_names ["weight", "type"]
UNWIND $rows AS r MATCH (a:M) WHERE a.id = r.source
                  MATCH (b:M) WHERE b.id = r.target CREATE …
  -> Execute error: Variable 'r' not found in chunk field_names ["weight", "type", "b.id", …]
MATCH (a:M), (b:M) UNWIND $rows AS r WITH a, b, r WHERE a.id = r.source …
  -> jalan, 0 baris (filter tidak pernah mengikat)
```

Yang **jalan**: `UNWIND` untuk node (`CREATE (n:T {col: r.col})`) dan untuk rel pola read
(`UNWIND $ids AS iid MATCH (a:T {key: iid})-[e:R]->(b:T) RETURN …`). Jadi masalahnya spesifik:
variabel hasil `UNWIND` tidak dapat dipakai dari dalam/ di belakang `MATCH` pada jalur tulis.

### Dampak & mitigasi

- `insert_edges` (P123.2) menulis satu pernyataan per tepi. Biaya: plan+execute per tepi, bukan
  per batch; *parse* tetap sekali karena teks pernyataannya konstan. Untuk formasi agen yang
  menulis ratusan tepi per giliran, ini jalur panas yang seharusnya bisa di-batch.
- Jalan pintas "resolve endpoint di Rust lalu tulis per tepi" tetap melewati transaksi/WAL —
  tidak ditempuh, lihat catatan P60.7.

### Langkah lanjut (usul, belum dikerjakan)

- Reproduce minimal di `akar-main` (2 node, 1 batch 2 tepi) dan telusuri `akar-binder` pada
  resolusi variabel `UNWIND` di dalam `MatchClause` → kemungkinan besar di jalur join/extend.
- Setelah jalan, `insert_edges` kembali ke bentuk batch dan `tests/test_embedding_api.rs`
  mengukurnya (jumlah pernyataan per batch).
- Catatan terkait: `ORDER BY` di path ini me-resolve ke **alias proyeksi**, bukan ke ekspresi
  (`ORDER BY a.id` gagal `Variable 'a' not found in chunk field_names`); `neighbors` memakai alias.

---

## F8 — 2026-09-19: verifikasi live DAE terhadap daemon produksi (fix `2464f40` Sulur) — TERVERIFIKASI, satu utas terbuka

**Ranah:** akar (daemon/DB produksi) ↔ sulur. **Status:** TERVERIFIKASI — DAE pass penuh + resume
berjalan tanpa kerusakan, schema DDL tanpa `DEFAULT` diterima. Utas yang masih terbuka: temuan
sampingan no.2 di bawah (batch besar tidak rentan di jalur DAE, kontras dengan F7 no.2 — layak
ditelusuri terpisah).
**Biner:** `~/.cargo/bin/akar_server.exe` v0.2.3 (dibangun 2026-09-18 23:34, HEAD `40415b9`) — sama dengan
yang dibuktikan sehat di F7 (copy DB +`mv wal.log` → listen).
**DB:** `~/.sulur/engine/sulur.db` (daemon live, pid 3268 / port 9876, sidecar token OK).
**Sumber:** verifikasi sisi kliem (repo Sulur, fix DDL `2464f40`) — replay DDL drive lewat
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

## F7 — 2026-09-18/19: replay WAL gagal di jalur **edge update** (`Edge index 0 out of range`) — kode TERATASI (`cd31526`), verifikasi live TERBUKA

**Ranah:** akar (storage / WAL replay). **Status:** TERATASI di tingkat kode — guard tulis (P114.1) mencegah WAL lahir tak-replayable dan mode salvage resmi (P114.2) menyediakan jalur pemulihan; verifikasi live ulang pada daemon produksi **belum dijalankan**, dan dua utas sampingan di bawah (kematian daemon senyap, batch tulis besar rentan) masih terbuka — karena itu finding ini belum boleh dihapus.
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
