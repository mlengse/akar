# Akar — Implementation Plan

> **Fungsi dokumen:** rencana kerja aktif (prioritas P0 → P2) + cara mengeksekusi
> dan memverifikasinya di lingkungan lain. Temuan ber-tanggal ada di `FINDINGS.md`;
> pekerjaan selesai dipindah ke `CHANGELOG.md`.

Terakhir diperbarui: 2026-09-17 · Konteks: audit Sulur ↔ Akar 2026-09-16
(daemon live 737 memori / 24.974 edge, biner `v0.2.1+11 (b0fbee1)`).

## Priority order

| ID | Item | Severity | Status | Menutup temuan |
|---|---|---|---|---|
| P1-MERGE-1 | Jalur cepat `MERGE … SET` harus match baris yang ada (+ param) | high | OPEN | F2 |
| P1-PERF-1 | Traversal rel table besar (anchored 1-hop) harus sub-detik | high | OPEN | F3 |
| P2-CASE-1 | Semantik case identifier: dokumentasikan + tes negatif | low | OPEN | F4 |
| P2-WAL-1 | Resiliensi WAL replay saat insert duplicate-PK | medium | OPEN | F5 |

## Prasyarat lingkungan

- Rust: `stable-x86_64-pc-windows-gnu` (atau MSVC di Linux — recipe di bawah untuk Windows tanpa MSVC).
- Toolchain Windows (validated): rustup stable + llvm-mingw UCRT (MartinStorsjo) + stub libgcc→libunwind.
- Repo: `akar-core/` workspace; test gate = `test [akar-core]` (2.071 tes, ~lama).

```bash
# Build daemon (perhatikan: paket namanya akar-server, HYPHEN)
cd akar-core
export RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld \
  -L <llvm-mingw>/x86_64-w64-mingw32/lib"
cargo build --release -p akar-server
# → target/release/akar_server.exe  (~2,5–6 menit; warm ~2,5 menit)
cargo fmt --all -- --check
cargo test --release -p akar-main --test test_ddl_errors      # targeted, 26 tes
cargo test --release -p akar-main                             # lebih luas
# gate penuh: cargo test --workspace  (≈ 2.071 passed baseline)
```

## Cara mereproduksi bug lewat daemon (dipakai untuk semua item di bawah)

Daemon = satu-satunya penulis DB; klien bicara TCP loopback dengan framing
`u32-LE length + JSON`, request `{"op":"query","token":"<hex>","query":"…","params":{…}}`.

```bash
# 1) daemon pada DB scratch
mkdir -p /tmp/akar-repro && ./target/release/akar_server.exe \
  --db /tmp/akar-repro --port 9876 --addr 127.0.0.1 --auth-token 00112233445566778899aabbccddeeff
# 2) kirim query (python; tanpa dependency akar)
python - <<'PY'
import json, socket, struct
HDR = struct.Struct("<I")
def q(cypher, params=None):
    req = {"op": "query", "token": "00112233445566778899aabbccddeeff", "query": cypher}
    if params is not None: req["params"] = params
    b = json.dumps(req, separators=(",", ":")).encode()
    s = socket.create_connection(("127.0.0.1", 9876)); s.settimeout(60)
    s.sendall(HDR.pack(len(b)) + b)
    n = HDR.unpack(s.recv(4))[0]; buf = b""
    while len(buf) < n: buf += s.recv(n - len(buf))
    return json.loads(buf)
print(q("CREATE NODE TABLE IF NOT EXISTS t(id INT64, v STRING, PRIMARY KEY(id))"))
print(q("CREATE (:t {id:1, v:'x'})"))
print(q("MERGE (m:t {id:1}) SET m.v = 'y'"))     # F2: duplicate PK (harusnya match)
print(q("MATCH (m:t) RETURN m.v AS v"))
PY
```

---

## P1-MERGE-1 — Jalur cepat MERGE harus match baris yang ada

**Temuan:** F2 · **File utama:** `akar-core/akar-main/src/connection/ddl.rs`
(arm `BoundStatement::BoundMerge`), `akar-core/akar-storage/src/table.rs`
(`TableCatalog::create_node_table`), `akar-binder/src/binder/mod.rs` (substitusi param).

**Langkah:**
1. **Konfirmasi hipotesis (30 menit).** Tulis tes kecil yang: buat `CREATE NODE TABLE t`,
   `CREATE (:t {id:1})`, lalu dari *instans katalog yang berbeda* lakukan
   `get_node_table_by_name("t").hash_index.lookup("1")` — cek apakah `None`.
   Bandingkan dengan `table_num_rows("t")` (yang membaca `storage_manager.table_catalog()`).
   Kalau `lookup` = `None` padahal baris ada → hipotesis (a) terkonfirmasi
   (index hidup di instance/clone yang berbeda); kalau `Some` → lanjut ke hipotesis (b).
2. **Perbaiki evaluasi PK untuk bentuk param.** `evaluate_constant_expr` harus
   menangani `Expression::Parameter` (atau pastikan substitusi
   `substitute_params_in_statement` benar-benar mengenai `BoundMerge.patterns[].node.properties`
   sebelum arm ini jalan — cek juga `prepared.bound_statement` vs `LogicalMerge`).
   Bukti kuat bahwa substitusi *tidak* sampai: varian literal memberi "Duplicate primary
   key", varian param memberi "NULL value" — artinya `Parameter` masih hidup di eksekusi.
3. **Perbaiki match PK** sehingga menemukan baris yang ada (sumber index yang sama dengan
   jalur INSERT), atau — bila perbaikan index berisiko besar — **delegasikan MERGE ke jalur
   planner** (`LogicalOperator::Merge` → `PhysicalMerge`, `map_update.rs`) yang terbukti
   benar, dan jadikan jalur cepat hanya sebagai shortcut untuk bentuk yang sudah tervalidasi.
4. **Tes regresi** (baru: `akar-core/akar-main/tests/test_merge.rs`):
   - `merge_literal_matches_existing_row` (tidak ada duplicate PK; `ON MATCH SET` berlaku),
   - `merge_param_matches_existing_row` (param, bukan NULL PK),
   - `merge_param_creates_when_absent` (PK = nilai param, bukan NULL),
   - `merge_is_idempotent_for_edges`: `MERGE (a)-[r:Connected]->(b)` dijalankan 3× pada
     pasangan sama → `count(r) == 1`.
   **Kriteria lulus:** semua hijau + `cargo fmt --check` bersih + gate tidak turun.

**Bukti dampak yang harus ikut sembuh:** setelah fix, ulangi skenario Sulur — daftar edge
tidak boleh bertambah saat upsert pasangan yang sama (`Connected` tumbuh hanya untuk
pasangan baru).

**Risiko:** menyentuh jalur tulis `Connection` → wajib jalankan gate penuh sebelum merge.

---

## P1-PERF-1 — Traversal rel table besar harus sub-detik

**Temuan:** F3 · **Ukuran:** 24.974 edge `Connected`, 737 node `Memory` (embedding 384-d).

**Langkah:**
1. **Baseline terukur** pada DB hasil unduh/seed (skrip di atas): catat waktu untuk
   1-hop anchored (typed & untyped), 2-hop, dan `RETURN count(r)` vs `RETURN b.id LIMIT 5`.
   Target: 1-hop anchored < 50 ms, `count(r)` < 200 ms pada 25k edge.
2. **Profil** jalur eksekusi: `akar-planner` (Extend/`recursiveextend.rs`) → apakah
   traversal memakai indeks adjacency (`fwd_adj`/`bwd_adj`) untuk node yang di-*anchor*,
   atau memindai seluruh rel; cek juga apakah baris *node* (termasuk kolom embedding)
   dikloning/dideserialisasi per hop padahal hanya `b.id` yang diproyeksikan.
3. **Kandidat perbaikan** (pilih berdasarkan profil; dari yang paling murah):
   - hindari materialisasi kolom besar saat traversal (projection pushdown ke rel/node),
   - gunakan indeks adjacency + `LIMIT` pushdown untuk pola anchored,
   - cache blok adjacency per node group bila memang di-scan per baris.
4. **Tes/benchmark**: tes integrasi baru yang men-seed 25k edge lalu menegakkan budget
   waktu 1-hop anchored (longgar, mis. < 500 ms agar tidak flaky di CI) — sebagai penjaga
   regresi. Sertakan `EXPLAIN` yang menunjukkan index dipakai.
5. **Batas RAM (wajib — lihat F6):** perbaikan harus membuktikan RSS **terbatas**, bukan
   hanya cepat. Target: query anchored 1-hop dan `get_all_connections`-style scan pada
   25k edge tidak boleh menaikkan RSS daemon lebih dari puluhan MB (bandingkan
   `WorkingSet64` sebelum/sesudah), dan tidak ada `memory allocation of … bytes failed`.
   Alokasi per-call 576 MiB–1,1 GiB (F6) adalah angka yang harus hilang.
6. **Verifikasi dampak di Sulur:** `sulur_dream_dae` selesai tanpa RPC timeout
   (lihat rencana Sulur P1-DAE-1).

**Catatan:** `AKAR` bisa diuji tanpa Sulur — semua data bisa di-seed lewat daemon scratch.

---

## P2-CASE-1 — Semantik case identifier

**Temuan:** F4 · **File:** spec + `akar-main/tests/` (tes negatif).

1. Putuskan & dokumentasikan aturan (saat ini: **case-sensitive** untuk node/rel table name,
   errornya `Bind error: Rel table 'CONNECTED' not found`). Tambahkan ke `SPEC.md`
   (§ bahasa/semantik identifier) + contoh benar/salah.
2. Tambahkan tes negatif yang mem-pin pesan error untuk salah case (node + rel), supaya
   perubahan tak sengaja pada normalisasi identifier ketahuan.
3. Pertimbangkan (opsional, kalau ingin kompatibel Kuzu): normalisasi ke identifier case
   asli saat bind, dengan pesan error yang menyebut nama terdaftar. **Jangan** ubah perilaku
   tanpa tes.

---

## P2-WAL-1 — Resiliensi WAL replay pada duplicate-PK insert

**Temuan:** F5 · **File:** `akar-storage` (WAL replay), `akar-main` (open path).

1. Reproduksi: DB scratch + WAL yang memuat dua insert PK sama untuk satu node table →
   server menolak start (`WAL recovery failed … Duplicate primary key value`).
2. Pilih kebijakan eksplisit dan dokumentasikan:
   - **(disarankan)** replay menerapkan *last-write-wins* untuk insert duplikat pada tabel
     node/rel dengan PK yang sama, sambil menulis `tracing::warn!` yang menyebut tabel+PK, **atau**
   - pertahankan fail-loud tetapi sediakan jalur pemulihan otomatis:
     `akar_server --repair-wal` / `akar-cli repair` yang membuang record duplikat
     (dengan backup WAL terlebih dahulu) lalu start.
3. Tes: (a) replay dengan duplikat → sesuai kebijakan yang dipilih, (b) data pra-crash tetap
   ada, (c) `--repair-wal` menghasilkan DB yang bisa dibuka + backup WAL tersimpan.
4. Sertakan catatan di `SPEC.md` §durability: apa yang dijamin replay dan apa yang tidak.

---

## Definisi selesai (per item)

1. Perbaikan kode + tes regresi yang gagal sebelum & lulus sesudah.
2. `cargo fmt --all -- --check` bersih; `cargo clippy -D warnings --all-targets` bersih
   (baseline repo).
3. Gate `test [akar-core]` dijalankan penuh (**tidak boleh turun** dari baseline 2.071 dan
   tidak boleh ada regresi baru).
4. Reproduksi lewat daemon scratch menunjukkan perilaku baru (tempel output sebelum/sesudah
   di `FINDINGS.md`; item selesai → pindah ke `CHANGELOG.md`).
5. Update `FINDINGS.md` (status) + `CHANGELOG.md` (entri `[Unreleased] → Fixed/Changed`).

## Lampiran — data pendukung (2026-09-16, terdokumentasi)

- `Connected` 972 → 24.974 edge dalam satu hari; laporan Sulur
  `SUPERSEDES: 11.608 new edges from 11.608 pairs checked` dan `13.818 dari 13.818` = **100 %**
  pasangan dianggap "baru" (indikasi kuat kegagalan match, konsisten dengan F2).
- `MATCH (m:Memory) RETURN count(m)` sempat 3,9 s untuk 737 baris (daemon sibuk, korelasi
  belum diisolasi — catat sebagai titik ukur tambahan saat P1-PERF-1).
