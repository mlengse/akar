# Akar — Findings

> **Fungsi dokumen:** jurnal temuan ber-tanggal (findings, incidents, audits,
> status verifikasi). Temuan yang sudah selesai dipindah ke `CHANGELOG.md`.
> Rencana kerja aktif ada di `implementation plan.md`. Bukan instruksi kerja.
>
> Asal temuan: audit Sulur ↔ Akar 2026-09-16 (daemon live `~/.sulur/engine/sulur.db`,
> biner `v0.2.1+11 (b0fbee1)`, DB 737 memori / 24.974 edge `Connected`).

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

Penyebab langsung: bug F1 (`CREATE NODE TABLE IF NOT EXISTS` mengulang storage — sudah
FIXED, lihat `CHANGELOG.md` `508328a`; tabel `DreamSession` di-wipe, id dipakai ulang, WAL
memuat dua insert `id=1`). Setelah F1 diperbaiki, urutan ini tidak bisa lagi tercipta lewat jalur itu —
**tetapi** mekanisme yang ada sekarang = server **menolak start** (fail-loud, bagus untuk
integritas) tanpa jalur pemulihan yang jelas di luar intervensi manual.

**Status:** OPEN (resiliensi) — rencana di `implementation plan.md` P2-WAL-1.
