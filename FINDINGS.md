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

## F17 — 2026-09-26: auto-checkpoint (threshold 16 MiB) menggelembungkan tulis berkelanjutan — TERVERIFIKASI

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

**Verifikasi (selesai 2026-09-26; A/B back-to-back, `RECALL=0`, `NO_CHECKPOINT=1`
knob ditambahkan di `tier_b.rs`).** store_batch 2000..6000 (4 chunk × 1000 row):

| chunk | baseline (cp ON) | NO_CHECKPOINT |
|---|---|---|
| 1000..2000 (ref) | 1,30 s | 1,44 s |
| 2000..3000 | **15,71 s** | 1,39 s |
| 3000..4000 | 1,31 s | 1,58 s |
| 4000..5000 | 1,76 s | 1,68 s |
| 5000..6000 | **31,30 s** | 2,20 s |
| total 2000..6000 | **50,08 s** | **6,84 s (7,3×)** |

Spike muncul di chunk yang sama dengan observasi awal (2000..3000 dan
5000..6000 ≈ WAL melewati 16 MiB tiap ~1,3–2k row) dan **hilang** saat
auto-checkpoint dimatikan; base chunk non-spike di kedua sisi tetap rata
(~1,3–1,8 s). `store_single` tak terdampak (63,5 s vs 59,0 s) — konsisten
dengan tesis spike = sinyal checkpoint di jalur commit, bukan jalur tulis.

**Dampak.** Bench 10.000 row target (`P6-BENCH-1`) akan pungut biaya
checkpoint beberapa kali. Opsi mitigasi bila terbukti: (a) `CHECKPOINT`
eksplisit di sela batch dengan ambang dibesarkan, (b) knob ekspos di harness,
(c) lewati ambang saat tulis beruntun dalam satu transaksi.

**Keputusan pengangkatan (2026-09-26):** **bukan task** — biarkan sebagai
temuan dengan mitigasi default tercatat. Alasan: workload pemantik telah
memenuhi target (store 10k `P6-FORM-1` selesai ~2,5 menit < 5 menit); `store_single`
tak terdampak; produksi jauh di bawah skala yang menyakitkan (DB audit: 737
memori). **Mitigasi default yang dipilih: (a)** — caller ingest berkelanjutan
(harness bench, sulur dream batch, formasi) mengeluarkan `CHECKPOINT` eksplisit
di batas batch dan, bila perlu, menaikkan `config.checkpoint_threshold`
(> 16 MiB) agar sinyal auto tidak menyela di tengah batch. Perubahan engine
jauh-jauh hari (c) hanya bila profil produksi menunjukkan tail terpola; sampai
itu, ambang auto 16 MiB dipertahankan sebagai jaring pengaman. Keputusan ini
bisa direvisi bila `P6-RECALL-1` atau bench formasi berikutnya menarik biaya
checkpoint sebagai variabel dominan.
