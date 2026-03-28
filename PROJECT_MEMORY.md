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

## Recent session note (2026-03-28, lazy parameter-name recovery)

- Added a lazy procedure-parameter cache in `src/proc_params.rs` so `ArkPdbReader` can recover real parameter names from DBI module symbol streams without changing `ark_pdb_open` or `ark_pdb_find_symbol_rva`.
- `Session` now keeps the original PDB path and reopens the PDB only on the first `ark_pdb_find_class_functions` call, where it walks `S_GPROC32` / `S_LPROC32` scopes and reads direct-child `S_LOCAL` parameter records. `S_REGREL32` is kept as a weaker fallback when no `S_LOCAL` names exist.
- Overload selection in `src/field_list.rs` now prefers the decorated candidate whose recovered parameter signature best matches the TPI parameter list instead of always taking the first public-symbol match.
- Public-symbol decorated names are still the only names used for `decorated_name`; the module-symbol scan is used to improve parameter names, not to change the fast symbol RVA path.
- Added unit tests covering overload disambiguation and hidden leading parameter alignment, and updated the docs to state clearly that startup offset lookup still avoids the module-symbol scan.

## Recent session note (2026-03-28, normalized overload matching)

- `src/proc_params.rs` now normalizes parameter type spellings before it ranks module-symbol candidates for a function.
- The overload scorer now treats these as strong or relaxed matches instead of outright mismatches:
  - `class UCanvas const*` vs `UCanvas const*`
  - `float &` vs `float&`
  - `FString const&` vs `const FString&`
- The public decorated-name path still stays lazy and cached behind `ark_pdb_find_class_functions`.
- `ark_pdb_open` and `ark_pdb_find_symbol_rva` were intentionally left unchanged, so the fast startup offset lookup path still does not pay for this work.
- Added unit tests for normalized type spelling and relaxed `const` placement matching in `src/proc_params.rs`.
