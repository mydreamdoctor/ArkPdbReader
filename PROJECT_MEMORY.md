# ArkPdbReader Project Memory

Last updated: 2026-03-25 (Australia/Brisbane)
Repository: `/home/lachlan/DEVELOPMENT/CSHARPAPI/ArkPdbReader`

## Purpose

ArkPdbReader is the shared native PDB reader used by ArkSdkGen.

Primary outputs:
- `libark_pdb_reader.so` for Linux-native builds
- `ark_pdb_reader.dll` for Windows-targeted builds
- C API contract in `include/ark_pdb_reader.h`
- C++ RAII wrapper in `include/ark_pdb_reader.hpp`

Main downstream contract:
- `ArkSdkGen` links this library as its native PDB/symbol backend on both Windows and Linux.

## Current Architecture Snapshot

### Implementation
- Rust crate using `microsoft/pdb-rs`.
- Exposes a C ABI for session open/close, type scanning, class layout extraction, member-function enumeration, symbol listing, and symbol RVA lookup.
- C++ wrapper provides safer session-oriented access for ArkSdkGen.

### Key source layout
- `src/lib.rs`: exported FFI surface
- `src/session.rs`: session lifecycle and stream ownership
- `src/symbol_catalog.rs`: cross-platform symbol enumeration and demangling support
- `include/ark_pdb_reader.h`: C API contract
- `include/ark_pdb_reader.hpp`: C++ wrapper used by ArkSdkGen
- `docs/api-contract.md`: contract notes for consumers
- `docs/integration-guide.md`: ArkSdkGen integration guidance

## Build/Run Notes

### Build
- Linux-native: `cargo build --release`
- Linux-hosted Windows cross-build: `cargo build --release --target x86_64-pc-windows-gnu`

### Expected artifacts
- `target/release/libark_pdb_reader.so`
- `target/x86_64-pc-windows-gnu/release/ark_pdb_reader.dll`
- `target/x86_64-pc-windows-gnu/release/libark_pdb_reader.dll.a`

## Practical Reload Checklist For Future Sessions

1. Read this file first.
2. Confirm the current public contract in:
   - `include/ark_pdb_reader.h`
   - `include/ark_pdb_reader.hpp`
   - `src/lib.rs`
3. If ArkSdkGen integration is relevant, also inspect:
   - `docs/integration-guide.md`
   - `../ArkSdkGen/src/native/pdb_reader_pdbrs.cpp`
   - `../ArkSdkGen/sdk-gen.proj`
4. Recall durable `memory` with project `CSHARPAPI` and tags including `repo:ArkPdbReader`.

## Recent session note (2026-03-25, cross-platform symbol API checkpoint)

- Added cross-platform symbol entry enumeration so ArkSdkGen no longer depends on a Windows-only reader for symbol export.
- `src/symbol_catalog.rs` now walks TPI, IPI, and public symbol streams.
- Public symbol names are demangled for display with `msvc-demangler`.
- `src/lib.rs`, `include/ark_pdb_reader.h`, and `include/ark_pdb_reader.hpp` now expose symbol-entry callbacks and a direct symbol RVA lookup path.
- ArkSdkGen now relies on this reader on both Windows and Linux.
- End-of-day pushed commit:
  - `0e45fe6` (`Add symbol catalog APIs for ArkSdkGen integration`)
