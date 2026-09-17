# Akar — Findings

> **Fungsi dokumen:** jurnal temuan ber-tanggal (findings, incidents, audits,
> status verifikasi). Temuan yang sudah selesai dipindah ke `CHANGELOG.md`.
> Rencana kerja aktif ada di `implementation plan.md`. Bukan instruksi kerja.
>
> Asal temuan: audit Sulur ↔ Akar 2026-09-16 (daemon live `~/.sulur/engine/sulur.db`,
> biner `v0.2.1+11 (b0fbee1)`, DB 737 memori / 24.974 edge `Connected`).

---

## F6 — Blow-up memori: daemon 4,3 GB RSS untuk DB 19 MB, scan rel mengalokasikan 576 MiB–1,1 GiB sekaligus → abort OOM

**Severity:** critical (proses daemon mati, memori host habis, klien kehilangan daemon)
**Tanggal:** 2026-09-17 · **Ranah:** akar (planner/processor/storage) — satu keluarga dengan F3

**Pengukuran live (2026-09-17, setelah insiden penulisan massal Sulur):**

| Metrik | Nilai |
|---|---|
| RSS `akar_server` | **4.315 MB** untuk DB **19 MB** (≈227× ukuran data) |
| Host (Windows, total 7.807 MB) | sisa bebas ~**1.131 MB** saat diukur (dilaporkan ~173 MB saat insiden) |
| `memory allocation of … bytes failed` di log daemon | **26** kejadian, antara lain `603979776` (**576 MiB**), `1207959552` (**1,125 GiB**), `4718592`, `24576` |

Pemicu: klien menarik **seluruh** rel dalam satu query
(`MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN … ORDER BY r.weight DESC` dengan
24.974 edge) — jalur yang sama dengan F3, tapi dampaknya RAM, bukan hanya waktu.
Setiap kegagalan alokasi = proses abort → klien kehilangan daemon dan harus spawn ulang.

**Dampak nyata (terverifikasi di Sulur):** inisialisasi engine Sulur memuat graf penuh saat
boot (`_load_from_store()` → `get_all_connections()`); pada graf sebesar ini proses gagal /
mematikan daemon → tool `sulur_*` UNAVAILABLE untuk sesi berjalan (Sulur `docs/FINDINGS.md`
#36G).

**Status:** OPEN — P1-PERF-1 **diperluas**: perbaikannya harus membatasi **RAM** dan waktu
(streaming/limit pushdown; jangan materialisasi seluruh rel + kolom embedding), dengan
kriteria lulus eksplisit pada RSS.

## F2 — `MERGE ... SET` (jalur cepat `Connection`) tidak pernah match baris yang ada

**Severity:** high (root fix BELUM; mematikan fase AFE Sulur + memicu ledakan edge)
**Tanggal:** 2026-09-16 · **Ranah:** akar (semantik MERGE)

**Repro (daemon live, tabel `Meta` PK `key`, baris sudah ada):**

| Statement | Hasil |
|---|---|
| `MERGE (m:Meta {key:'t'}) SET m.value='v'` | **gagal** `MERGE CREATE failed: index: Duplicate primary key value: 't'` → match meleset, lalu CREATE |
| `MERGE (m:Meta {key:$k}) SET m.value=$v` (param) | **gagal** `MERGE CREATE failed: page: NULL value not allowed for primary key column 'key'` |
| `MERGE (m:Meta {key:$k}) RETURN m.value` (tanpa SET) | **sukses** (match & create benar, jalur planner → `PhysicalMerge`) |
| `MATCH (m:Meta {key:$k}) SET m.value=$v` (param) | **sukses** |
| `CREATE (:Meta {key:$k, value:$v})` (param) | **sukses** |

**Kesimpulan:** hanya jalur cepat `Connection` — `akar-main/src/connection/ddl.rs`,
arm `BoundStatement::BoundMerge` (bentuk `MERGE … SET` tanpa `RETURN`) — yang salah.
Dua gejala, satu lokasi:

1. **Literal:** match lewat `table.hash_index.lookup(&pk_value_to_string(pv))` selalu
   `None` untuk baris yang ada → jatuh ke CREATE → duplicate PK.
2. **Param:** nilai PK dievaluasi dari ekspresi yang tidak ter-substitusi/ter-evaluasi
   (`evaluate_constant_expr` pada `Expression::Parameter`) → `Null` → "NULL value not
   allowed for primary key". (Jalur pipeline `map_ddl.rs`/`PhysicalMerge` sudah benar.)

**Hipotesis akar (perlu dikonfirmasi saat memperbaiki):**
- (a) *Index tidak hidup di instance yang dibaca.* `TableCatalog::create_node_table`
  (`akar-storage/src/table.rs:1306`) menyimpan **clone** `NodeTable` ke `node_tables`
  (`node_tables.insert(table_id, table.clone())`); kalau `hash_index` dimiliki per-instance
  (bukan `Arc`), mutasi insert bisa masuk ke instance lain dari yang di-`lookup` oleh
  jalur MERGE.
- (b) `hash_index` hanya dipelihara jalur INSERT (binder/ART), tidak oleh jalur
  MERGE/CREATE-pipeline, sehingga tabel yang diisi lewat MERGE tidak pernah terindeks.

**Dampak nyata (terverifikasi di Sulur):**
- Fase AFE Sulur (`afe._write_done_set`) memakai `MERGE (m:Meta {key:$k}) SET m.value=$v`
  → `Dream cycle failed: Execute error: MERGE CREATE failed … in table 'Meta'`
  (2026-09-16 12:45), membatalkan **seluruh siklus dream**. Mitigasi sementara di sisi
  Sulur: MATCH → SET/CREATE (commit `eb24fbf`).
- Upsert edge Sulur (`akar_store.upsert_connection`, `plugin.py` MERGE `Connected`)
  berisiko membuat edge **duplikat** setiap kali dipanggil, bukan memperbarui weight →
  selaras dengan kenaikan `Connected` 972 → 24.974 dalam satu hari.

**Status:** OPEN — rencana perbaikan di `implementation plan.md` P1-MERGE-1.

---

## F3 — Traversal rel table besar patologis (anchored >30 s)

**Severity:** high (kanal graf Sulur — DAE/PPR/ColBERT — efektif mati)
**Tanggal:** 2026-09-16 · **Ranah:** akar (planner/processor/storage)

**Pengukuran (daemon live, `Connected` = 24.974 edge):**

| Query | Waktu |
|---|---|
| `MATCH (a:Memory {id:1})-[r:Connected]->(b:Memory) RETURN count(r)` | **timeout > 25 s** |
| `MATCH (a:Memory {id:1})-[r:Connected]-(b:Memory) RETURN b.id LIMIT 5` | **timeout > 25 s** |
| `MATCH (a:Memory)-[r:Connected]->(b:Memory) RETURN a.id LIMIT 3` | **timeout > 30 s** |
| `MATCH (a:Memory {id:1})-[r]-(b:Memory) RETURN b.id LIMIT 5` (untyped) | 9,9 s (lambat tapi jalan) |
| `MATCH (a:MetadataTable)-[r:HAS_COLUMN]->(b:MetadataColumn) RETURN count(r)` (rel kecil, 0 baris) | 0,05 s |
| `MATCH (m:Memory) RETURN count(m)` (737 baris) | 3,9 s (saat daemon sibuk) |

Rel kecil 0,05 s vs `Connected` timeout → biaya **proporsional ukuran rel** dengan
konstanta sangat buruk (bukan overhead global). Anker tunggal pun tidak membantu
(`{id:1}` exact) → dugaan: traversal tidak memakai indeks adjacency / materialisasi
baris node (termasuk `embedding` 384-d) per hop, atau full-scan rel per baris input.

**Dampak:** `dae.py` (sulur) men-scan seluruh edge → RPC timeout 120 s →
`sulur_dream_dae` gagal (`RuntimeError: akar-server RPC failed: timed out`), siklus
dream tidak pernah tuntas.

**Status:** OPEN — rencana di `implementation plan.md` P1-PERF-1.

---

## F4 — Identifiers case-sensitive (dan tidak terdokumentasi)

**Severity:** low (footgun; sudah menggigit Sulur)
**Tanggal:** 2026-09-16 · **Ranah:** akar (semantik bahasa) + sulur (typo)

- `MATCH (a:Memory)-[r:CONNECTED]-(b:Memory) …` → `Bind error: Rel table 'CONNECTED' not found`
  (tabel terdaftar sebagai `Connected`). Nama node table sama: `Memory`, `Meta`, `DreamSession` case-sensitive.
- Sulur `dae.py:94` (dan `:239`) memakai `[r:CONNECTED]` → query bind-error (bug sisi Sulur).
- **Status:** OPEN — keputusan: dokumentasikan aturan case di SPEC + tambahkan tes
  negatif yang mem-pin pesan errornya (`implementation plan.md` P2-CASE-1); perbaikan typo Sulur ada di
  rencana Sulur.

---

## F5 — WAL replay menolak start pada insert duplicate-PK

**Severity:** medium (resiliensi; pernah membuat server tidak bisa start)
**Tanggal:** 2026-09-16 · **Ranah:** akar (recovery)

Log daemon 2026-09-16 12:14–12:19 (3×):

```
Failed to open database at '…\sulur.db': WAL recovery failed (database may need manual
repair): WAL recovery insert failed: index: Duplicate primary key value: '1' in table
'DreamSession'. Refusing to start with an empty database — check the WAL.
```

Penyebab langsung: bug F1 (tabel `DreamSession` di-wipe, id dipakai ulang, WAL memuat dua
insert `id=1`). Setelah F1 diperbaiki, urutan ini tidak bisa lagi tercipta lewat jalur itu —
**tetapi** mekanisme yang ada sekarang = server **menolak start** (fail-loud, bagus untuk
integritas) tanpa jalur pemulihan yang jelas di luar intervensi manual.

**Status:** OPEN (resiliensi) — rencana di `implementation plan.md` P2-WAL-1.

---

## F1 — `CREATE NODE TABLE IF NOT EXISTS` membuat ulang storage (silent data loss) — FIXED

**Severity:** high (data loss senyap) · **Tanggal:** 2026-09-16 · **Status:** SELESAI → CHANGELOG `508328a`

Ringkas (detail lengkap di `CHANGELOG.md`):

- Arm `BoundCreateNodeTable`/`BoundCreateRelTable` di `akar-main/src/connection/ddl.rs`
  memanggil `database.create_node_table(...)` tanpa memeriksa `if_not_exists`;
  `TableCatalog::create_node_table` mengalokasikan `table_id` baru + me-remap
  `node_name_to_id` → seluruh baris tabel lama tak terjangkau, statement tetap "sukses".
- Repro (sebelum): create `IF NOT EXISTS` → insert → count 1 → ulangi DDL → **count 0**.
  Sesudah fix: count tetap 1, tabel tetap writable, plain duplicate tetap error.
- Tes `akar-main/tests/test_ddl_errors.rs` diperkuat (data ditulis **sebelum** DDL duplikat) →
  `cargo test --release -p akar-main --test test_ddl_errors`: 26 passed / 0 failed.
- Dampak nyata: setiap `sulur_dream_stats` menghapus `DreamSession`/`ConnectionHistory`/
  `DreamInsight` (telemetri selalu `sessions: 0`) dan memicu WAL replay failure (F5).
