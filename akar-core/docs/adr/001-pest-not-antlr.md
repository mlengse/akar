# ADR 001: Mengapa pest.rs (PEG) bukan ANTLR4

> **Status:** Accepted | **Date:** 2026-07-07 | **Last Updated:** 2026-07-19

## Context

Kùzu C++ menggunakan ANTLR4 untuk parsing Cypher. Untuk port Rust, kita perlu memilih parser generator yang idiomatic.

## Decision

Menggunakan **pest.rs** — parser expression grammar (PEG) untuk Rust.

## Alternatives Considered

| Alternatif | Kelebihan | Kekurangan |
|-----------|----------|-----------|
| **ANTLR4 Rust** | Sama dengan C++, grammar bisa di-share | Runtime berat, codegen Java, tidak idiomatic Rust |
| **nom** | Zero-copy, performa tinggi | Grammar kompleks sulit dibaca, verbose |
| **lalrpop** | LALR(1), error messages bagus | Tidak bisa parse Cypher (bukan LALR) |
| **pest.rs** ✅ | Grammar declarative, error reporting bagus, macro-based | Performa sedikit di bawah nom |

## Consequences

- Grammar ditulis ulang dari ANTLR4 `.g4` ke PEG `cypher.pest`
- Tidak bisa share grammar dengan C++ — maintain dua grammar paralel
- PEG menangani whitespace, precedence, dan associativity secara eksplisit
- ~63 test parser memverifikasi paritas dengan C++ grammar
