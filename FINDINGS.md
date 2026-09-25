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

## F17 — 2026-09-26: auto-checkpoint (threshold 16 MiB) menggelembungkan tulis berkelanjutan — TERBUKA

**Gejala.** Bench `sulur/benchmarks/cpp_vs_rust/rust/src/bin/tier_b.rs` (Tier B,
GPL) yang menulis batched 1000-row ke `Memory` (2× `FLOAT[768]` per row ≈
12 KB) menunjukkan pola: chunk 1000..2000 dan 3000..4000 serta 4000..5000
berjalan ~1.2–1.75 ms/row, tetapi 2000..3000 = **9.4 s** dan 5000..6000 =
**20.2 s** per chunk. Spike-periodik ini melonjak dengan ukuran DB (yang kedua
lebih besar pada DB 6000-row) — bukan pertumbuhan monotonik O(n).

**Akar dugaan (verifikasi belum dilakukan — butuh `NO_CHECKPOINT` probe).**
`SystemConfig::default().checkpoint_threshold` = 16 MiB (P128, menutup F15)
berarti sinyal auto-checkpoint dikirim setiap WAL > 16 MiB
(`connection/query.rs:509-531`). Dengan ~12 KB/row, ambang terlampaui tiap
~1.3–2k baris → drain + tulis ulang mirror kolom semasa chunk sedang menulis →
spike menghantam tepat di tengah chunk. 20.2 s di DB 6000-row konsisten dengan
biaya checkpoint yang tumbuh seiring ukuran basis instalasi.

**Konteks & batas ranah.** Ini bukan pelebaran ulang F15 (yang hanya memilih
default `-1` vs 16 MiB — latensi per-tulis memang membaik). Ini efek residu
pilihan itu pada **workload tulis berkelanjutan** (ingest bertahap, bench
formasi) di mana checkpoint bukan per-op melainkan lonjakan deterministik yang
membunuh latensi tail. Jalur tulis Sulur (`P6-FORM-1`) terindikasi sehat;
spike bukan dari sana.

**Verifikasi yang dibutuhkan.** Jalankan bench store_batch dengan
`auto_checkpoint: false, checkpoint_threshold: 0` → spike harus hilang (chunk
semua ~1.2–1.75 ms/row). Konfirmasi menutup/membuka hipotesis di atas.

**Dampak.** Bench 10.000 row target (`P6-BENCH-1`) akan pungut biaya
checkpoint beberapa kali. Opsi mitigasi bila terbukti: (a) `CHECKPOINT`
eksplisit di sela batch dengan ambang dibesarkan, (b) knob ekspos di harness,
(c) lewati ambang saat tulis beruntun dalam satu transaksi.
