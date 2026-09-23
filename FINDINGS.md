# Akar — Findings

> **Fungsi dokumen:** temuan ber-tanggal (findings, incidents, audit) yang
> **belum diangkat menjadi task** di `implementation plan.md`. Begitu sebuah
> temuan dibuatkan task `P###`, entri di sini **dihapus** — task itu rumahnya,
> dan provenance-nya ditulis di task sebagai "Temuan: <id>". Begitu task-nya
> selesai, riwayatnya hidup di `CHANGELOG.md`. Karena itu berkas ini **bukan
> arsip**: tidak ada entri yang bertahan setelah punya task. Bukan rencana
> (→ `implementation plan.md`), bukan instruksi kerja (→ `AGENTS.md`), bukan
> fakta arsitektur & metrik (→ `SPEC.md`).
>
> Asal temuan terlama: audit Sulur ↔ Akar 2026-09-16 (daemon live
> `~/.sulur/engine/sulur.db`, biner `v0.2.1+11 (b0fbee1)`, DB 737 memori /
> 24.974 edge `Connected`).

**Tidak ada temuan terbuka tanpa task.** Seluruh temuan yang sebelumnya tinggal
di berkas ini sudah punya task di `implementation plan.md`:

| Temuan | Task |
|--------|------|
| F16 — `ORDER BY <alias proyeksi>` gagal saat chunk hasil kosong | `P130` |
| F14 — variabel `UNWIND` tak terpakai di jalur tulis pola `MATCH` | `P131` |
| #36I (Sulur) — `DELETE`/`SET` pada pola rel tak bisa dialamatkan per-edge | `P132` |
| F7 — replay WAL gagal di jalur edge update | `P133` (verifikasi live) · `P134` (dua utas sampingan) |
| F8 — verifikasi live DAE terhadap daemon produksi | `P134` (sisa utas); hasil verifikasi → `CHANGELOG.md` |
