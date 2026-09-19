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

## F9 — 2026-09-19: agenda lanjut terpilih (investigasi F7 + hardening kematian senyap daemon) — RENCANA

**Ranah:** akar (storage/WAL + daemon lifecycle). **Status:** RENCANA — belum dikerjakan;
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

## F7 — 2026-09-18/19: replay WAL gagal di jalur **edge update** (`Edge index 0 out of range`) — TERBUKA

**Ranah:** akar (storage / WAL replay). **Status:** TERBUKA — jalur replay edge `SET` belum tertangani.
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
