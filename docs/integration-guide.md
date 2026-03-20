# ArkPdbReader — Integration Guide

This document describes how to integrate ArkPdbReader into a C++ CMake project.

ArkPdbReader is a Rust library with a C API and C++ RAII wrapper. Your project
links against the pre-built shared or static library and includes the provided
headers. No Rust knowledge is needed beyond running `cargo build`.

---

## 1. Prerequisites

- Rust toolchain installed (1.75+). Install via <https://rustup.rs>.
- The ArkPdbReader source directory, either as a git submodule or a sibling
  checkout:
  ```
  your-project/
  ├── your-code/
  └── ArkPdbReader/
  ```

---

## 2. Build ArkPdbReader

```bash
cd ArkPdbReader
cargo build --release
```

Output:
- Linux: `target/release/libark_pdb_reader.so` (shared) and `target/release/libark_pdb_reader.a` (static)
- Windows: `target/release/ark_pdb_reader.dll` and `target/release/ark_pdb_reader.dll.lib` (import lib)

---

## 3. CMake integration

ArkPdbReader ships a `CMakeLists.txt` that creates an imported target called
`ark-pdb-reader`. Add it to your project:

```cmake
# Point to the ArkPdbReader source directory.
set(ARK_PDB_READER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../ArkPdbReader"
    CACHE PATH "Path to ArkPdbReader source directory")

# If you already ran `cargo build --release`, skip the automatic Rust build:
set(ARK_PDB_READER_BUILD_RUST OFF CACHE BOOL "Skip automatic cargo build")

add_subdirectory("${ARK_PDB_READER_DIR}" ark-pdb-reader-build)

# Link your target against it:
target_link_libraries(my_target PRIVATE ark-pdb-reader)
```

This automatically adds the correct include directories for the C and C++
headers.

If you set `ARK_PDB_READER_BUILD_RUST=ON` (the default), CMake will run
`cargo build --release` for you during the build step.

---

## 4. Using the C++ wrapper

```cpp
#include <ark_pdb_reader.hpp>
#include <iostream>

int main() {
    // Open a PDB file.
    ark::PdbSession pdb("/path/to/Game.pdb");
    if (!pdb) {
        std::cerr << "Failed to open PDB\n";
        return 1;
    }

    // Enumerate type entries with cached class/struct kind.
    pdb.listTypeEntries([](const ark::TypeEntryView& entry) {
        std::cout << entry.name << " ("
                  << (entry.kind == ark::TypeKind::Struct ? "struct" : "class")
                  << ")\n";
        return true;  // return false to stop early
    });

    // Get class layout (members).
    auto layout = pdb.getClassLayout("ACharacter");
    if (layout) {
        std::cout << "Size: " << layout.totalSize() << " bytes\n";
        std::cout << "Base: " << layout.baseClass() << "\n";
        for (int i = 0; i < layout.memberCount(); i++) {
            char name[256], type[512];
            layout.getMemberName(i, name, sizeof(name));
            layout.getMemberType(i, type, sizeof(type));
            std::cout << "  +" << layout.getMemberOffset(i)
                      << "  " << type << "  " << name << "\n";
        }
    }

    // Get member functions.
    auto fns = pdb.getClassFunctions("ACharacter");
    if (fns) {
        for (int i = 0; i < fns.count(); i++) {
            char name[256], ret[512];
            fns.getName(i, name, sizeof(name));
            fns.getReturnType(i, ret, sizeof(ret));
            std::cout << ret << " " << name << "("
                      << fns.paramCount(i) << " params)"
                      << (fns.isVirtual(i) ? " virtual" : "")
                      << (fns.isStatic(i) ? " static" : "")
                      << (fns.isConst(i) ? " const" : "")
                      << "\n";
        }
    }

    // Look up a symbol RVA by its decorated name.
    uint64_t rva = 0;
    if (pdb.findSymbolRVA("?MyFunc@ACharacter@@QEAAXXZ", rva)) {
        std::cout << "RVA = 0x" << std::hex << rva << "\n";
    }
}
```

---

## 5. Using the C API directly

If you are not using C++, the C API in `include/ark_pdb_reader.h` provides the
same functionality with opaque handles:

```c
#include <ark_pdb_reader.h>
#include <stdio.h>

int main() {
    ArkPdbSession* session = ark_pdb_open("Game.pdb");
    if (!session) return 1;

    ArkLayoutHandle* layout = ark_pdb_find_class_layout(session, "ACharacter");
    if (layout) {
        printf("Size: %d bytes\n", ark_pdb_layout_get_total_size(layout));
        printf("Members: %d\n", ark_pdb_layout_get_member_count(layout));
        ark_pdb_layout_free(layout);
    }

    ark_pdb_close(session);
}
```

See [`api-contract.md`](api-contract.md) for the complete behaviour contract
covering all functions, buffer sizes, error handling, and thread safety.

---

## 6. Deploying

**Linux:** Ship `libark_pdb_reader.so` alongside your binary, or install it to
a directory on `LD_LIBRARY_PATH`. If you link statically against
`libark_pdb_reader.a`, no runtime dependency is needed.

**Windows:** Ship `ark_pdb_reader.dll` alongside your executable.

---

## 7. API reference

- [`api-contract.md`](api-contract.md) — complete behaviour contract
- [`../include/ark_pdb_reader.h`](../include/ark_pdb_reader.h) — C API with inline docs
- [`../include/ark_pdb_reader.hpp`](../include/ark_pdb_reader.hpp) — C++ wrapper with inline docs
