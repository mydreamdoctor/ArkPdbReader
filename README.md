# ArkPdbReader

> **License:** This project is source-available under a [custom non-commercial license](LICENSE). You may view, modify, and use it in open-source projects with attribution. Commercial use is not permitted.

High-performance PDB reader for Unreal Engine game PDB files. Extracts class
layouts, member functions, and symbol RVAs from PDB files that the LLVM and DIA
backends struggle with.

---

## What this is

A Rust library that reads PDB files by parsing the **CodeView TPI stream**
directly — the same raw data the Windows DIA SDK uses.

The library exposes a **C API** (`include/ark_pdb_reader.h`) and a **C++ RAII
wrapper** (`include/ark_pdb_reader.hpp`) so it can be consumed from any C or
C++ project without any Rust knowledge required.

### Why not LLVM?

LLVM's `IPDBSession::findAllChildren` API performs repeated full-PDB scans and
cannot reliably walk `LF_FIELDLIST` records in large game PDBs.  The result is
0 members and 0 functions for every class, making it unusable for real work.

### Why direct TPI parsing?

The TPI stream is a flat packed array of CodeView type records.  One sequential
pass (O(N) ≈ 1–3 s for the full Ark PDB) builds a complete
`name → TypeIndex` hash map.  Every subsequent lookup is O(1).  Member and
function extraction for any class is then a single O(F) pass over its field
list — no repeated scans.

---

## Building

The library is written entirely in Rust. No C or C++ compiler is needed to
build the library itself — only to build your own project that links against it.

**Requires:** Rust 1.75+ — install from <https://rustup.rs>.

### Linux (native)

```bash
cargo build --release
```

Outputs (all produced in a single build):
| File | Description |
|------|-------------|
| `target/release/libark_pdb_reader.so` | Shared library (1.1 MB) — link dynamically, ship alongside your binary |
| `target/release/libark_pdb_reader.a` | Static archive (29 MB) — link statically, no runtime dependency |

### Windows (native, from a Windows host)

```bash
cargo build --release
```

Outputs:
| File | Description |
|------|-------------|
| `target/release/ark_pdb_reader.dll` | Shared library — ship alongside your executable |
| `target/release/ark_pdb_reader.dll.lib` | Import library — used by the linker at build time (MSVC) |

### Windows (cross-compiled from Linux)

Requires the `x86_64-pc-windows-gnu` target and a MinGW-w64 linker:

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

Outputs appear in `target/x86_64-pc-windows-gnu/release/`:
| File | Description |
|------|-------------|
| `ark_pdb_reader.dll` | Shared library |
| `libark_pdb_reader.dll.a` | Import library (MinGW format) |

### CMake integration

The included `CMakeLists.txt` creates an imported `ark-pdb-reader` target that
handles platform differences automatically. It can optionally run `cargo build`
for you during the CMake build step:

```cmake
add_subdirectory(path/to/ArkPdbReader)
target_link_libraries(my_target PRIVATE ark-pdb-reader)
```

Set `ARK_PDB_READER_BUILD_RUST=OFF` if you prefer to run `cargo build` yourself
before configuring CMake. See [`docs/integration-guide.md`](docs/integration-guide.md)
for full details.

---

## Quick start (C++)

```cpp
#include <ark_pdb_reader.hpp>
#include <iostream>

int main() {
    ark::PdbSession pdb("/path/to/ShooterGame.pdb");
    if (!pdb) { std::cerr << "open failed\n"; return 1; }

    // List UE-style type entries with cached kind.
    pdb.listTypeEntries([](const ark::TypeEntryView& entry) {
        std::cout << entry.name << " (" 
                  << (entry.kind == ark::TypeKind::Struct ? "struct" : "class")
                  << ")\n";
        return true;
    });

    // Get class layout
    auto layout = pdb.getClassLayout("AGameModeBase");
    if (layout) {
        std::cout << "Size: " << layout.totalSize() << "\n";
        for (const auto& m : layout.members())
            std::cout << "  +" << m.offset << "  " << m.typeName << "  " << m.name << "\n";
    }

    // Get member functions
    auto fns = pdb.getClassFunctions("AGameModeBase");
    if (fns) {
        for (const auto& f : fns.functions()) {
            std::cout << f.returnType << " " << f.name << "(...)\n";
            std::cout << "  // " << f.decoratedName << "\n";
        }
    }
}
```

---

## Integration

See [`docs/integration-guide.md`](docs/integration-guide.md) for a detailed
walkthrough of adding ArkPdbReader to a C++ CMake project.

---

## API reference

- [`docs/api-contract.md`](docs/api-contract.md) — complete behaviour contract
- [`include/ark_pdb_reader.h`](include/ark_pdb_reader.h) — C API with inline docs
- [`include/ark_pdb_reader.hpp`](include/ark_pdb_reader.hpp) — C++ wrapper with inline docs

---

## What is and isn't extracted

**Extracted:**
- All UE-style class and struct names (filterable by any caller)
- Direct base class name
- Total struct size in bytes
- All instance data members: name, C++ type name, byte offset, size
- All member functions: short name, decorated name, return type, parameters
  (name + type), `isVirtual`, `isStatic`, `isConst` flags

**Not extracted (current version):**
- Static data members (no instance offset)
- Operator overloads and constructors/destructors (intentionally excluded)
- Full template argument expansion (template names are included verbatim)
- Per-parameter names from PDB (parameter names are `param0`, `param1`, ...
  since large game PDBs do not reliably store them)
