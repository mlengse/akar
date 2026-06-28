# Benchmark Baseline — Kuzu C++ (Referensi)

Dokumen ini mencatat metodologi benchmark untuk Kuzu C++ sebagai baseline performa
sebelum refaktor ke Rust. Benchmark akan dijalankan setelah environment build lengkap.

---

## System Spec

| Item | Value |
|------|-------|
| OS | Windows |
| CPU | (auto-detected) |
| RAM | (auto-detected) |
| Storage | SSD (expected) |
| Rust | cargo 1.96.0, target x86_64-pc-windows-gnu |
| C++ Toolchain | MinGW-w64 gcc 14.2.0 (C:\mingw64) |
| CMake | Belum terinstal |

---

## Build C++ Kuzu + Benchmark Tool

```bash
# Install CMake & Ninja
winget install Kitware.CMake
pip install ninja

# Build Kuzu release with benchmark
cd kuzu
mkdir build/release
cmake -B build/release -G Ninja ^
    -DCMAKE_BUILD_TYPE=Release ^
    -DBUILD_BENCHMARK=TRUE ^
    -DBUILD_SHELL=OFF ^
    -DBUILD_SINGLE_FILE_HEADER=OFF ^
    -DAUTO_UPDATE_GRAMMAR=OFF .
cmake --build build/release

# Output binary: build/release/tools/benchmark/kuzu_benchmark.exe
```

---

## Siapkan Dataset (Tinysnb — Test Kecil)

Dataset terkecil (`dataset/tinysnb/`) bisa langsung digunakan tanpa serialisasi.

Untuk benchmark yang lebih representatif, gunakan LDBC SNB dataset:

```bash
# Generate LDBC SNB SF-1 dataset
python benchmark/serializer.py dataset/ldbc-sf01/ build/ldbc-sf01.kz
```

---

## Jalankan Benchmark

### 1. C++ Benchmark Tool (Mikrobenchmark)

```bash
# Single query benchmark
build/release/tools/benchmark/kuzu_benchmark ^
    --dataset=dataset/tinysnb ^
    --benchmark=benchmark/queries/example/example.benchmark ^
    --warmup=3 --run=10 --thread=4 --bm-size=4096

# Directory of benchmarks
build/release/tools/benchmark/kuzu_benchmark ^
    --dataset=build/ldbc-sf01.kz ^
    --benchmark=benchmark/queries/ldbc-sf100/filter ^
    --warmup=3 --run=10 --thread=8 --bm-size=8192
```

### 2. Python Benchmark Runner (Makrobenchmark)

```bash
pip install kuzu psutil
python benchmark/benchmark_runner.py --help
```

### 3. ClickBench

```bash
cd benchmark/click
bash run.sh  # Download hits.csv, load, run 43 queries
```

### 4. LSQB (LDBC Social Network Benchmark)

```bash
cd benchmark/lsqb
python benchmark_runner.py <dataset_path>
```

---

## Metrik yang Diukur

| Metrik | Deskripsi | Tools |
|--------|-----------|-------|
| Query latency (ms) | Waktu eksekusi per query (rata-rata 10 run) | kuzu_benchmark |
| Throughput (qps) | Queries per second | benchmark_runner.py |
| Memory usage (MB) | Peak memory selama benchmark | OS tools |
| Buffer pool hits | Cache hit rate | kuzu_benchmark --profile |

---

## Query Categories

| Kategori | Contoh | Jumlah Query |
|----------|--------|-------------|
| Filter | `WHERE property < X` | ~10 |
| Aggregation | `RETURN count(*), sum(prop)` | ~5 |
| Join | `MATCH (a)-[r]->(b) RETURN ...` | ~10 |
| Scan | `MATCH (n) RETURN n.prop` | ~8 |
| Graph Algo | PageRank, WCC, Louvain | ~10 |
| Shortest Path | BFS, weighted shortest | ~5 |
| Order By / Limit | `ORDER BY ... LIMIT` | ~5 |
| Recursive Join | Multi-hop traversal | ~8 |

**Total**: ~60+ queries across all categories

---

## Expected Performance (Order of Magnitude)

> Dari dokumentasi Kuzu untuk dataset sedang (LDBC SF-1 ~2GB):

| Query Type | Latency (C++) | Target Rust (Fase 12) |
|------------|---------------|----------------------|
| Point lookup | < 1ms | < 2ms |
| Simple filter | 1-5ms | 2-10ms |
| Aggregation | 5-20ms | 10-30ms |
| 2-hop join | 10-50ms | 20-80ms |
| 3+ hop join | 50-200ms | 100-400ms |
| Graph algo (WCC) | 100-500ms | 200-800ms |
| Bulk insert | 100K rows/s | 50K rows/s |

> **Target**: Rust dalam 2x performa C++ untuk Fase 1, dalam 1.5x untuk Fase 12.

---

## Catatan

- Baseline harus diambil sebelum perubahan signifikan pada kode Rust
- Benchmark diulang setelah setiap fase refaktor selesai
- Gunakan hardware yang sama untuk semua pengukuran
- Catat suhu CPU dan background processes saat benchmark
