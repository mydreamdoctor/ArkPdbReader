# ArkPdbReader — ArkSdkGen Integration Guide

This document describes exactly how to replace the current LLVM PDB backend
(`pdb_reader_llvm.cpp`) in ArkSdkGen with ArkPdbReader.

No changes to the ArkSdkGen public API (`class_layout.h`, `pdb_reader.h`, or
any higher-level code) are required.

---

## 1. Prerequisites

- Rust toolchain installed (1.75+). Install via <https://rustup.rs>.
- The `ArkPdbReader` repo checked out alongside `ArkSdkGen`:
  ```
  /home/lachlan/DEVELOPMENT/CSHARPAPI/
  ├── ArkSdkGen/
  └── ArkPdbReader/     ← new sibling
  ```

---

## 2. Build ArkPdbReader

```bash
cd /home/lachlan/DEVELOPMENT/CSHARPAPI/ArkPdbReader
cargo build --release
```

Output:
- `target/release/libark_pdb_reader.so`  — shared library (Linux)
- `target/release/libark_pdb_reader.a`   — static archive

---

## 3. Add a new PDB reader backend to ArkSdkGen

Create `ArkSdkGen/src/native/pdb_reader_pdbrs.cpp`. This file implements the
existing `ark::PdbReader` class (declared in `pdb_reader.h`) using the
ArkPdbReader C++ wrapper.

### Minimal template

```cpp
// src/native/pdb_reader_pdbrs.cpp
// Linux-only. Implements ark::PdbReader via libark_pdb_reader.so.

#include "pdb_reader.h"
#include "logger.h"

#ifndef _WIN32

#include <ark_pdb_reader.hpp>  // C++ wrapper
#include <string>
#include <vector>

namespace ark {

struct PdbReader::Impl {
    ::ark::PdbSession session;  // the library's PdbSession wrapper
};

PdbReader::PdbReader()  = default;
PdbReader::~PdbReader() { Close(); }

bool PdbReader::Open(const std::string& pdbPath) {
    if (m_initialized) return true;
    auto impl = std::make_unique<Impl>();
    impl->session = ::ark::PdbSession(pdbPath);
    if (!impl->session) {
        Logger::Error("ArkPdbReader: failed to open {}", pdbPath);
        return false;
    }
    m_impl = std::move(impl);
    m_initialized = true;
    Logger::Info("PDB loaded via ArkPdbReader: {}", pdbPath);
    return true;
}

void PdbReader::Close() {
    m_impl.reset();
    m_initialized = false;
}

bool PdbReader::TypeExists(const std::string& typeName,
                           std::string* outResolvedName) {
    if (!m_initialized) return false;
    if (outResolvedName) {
        return m_impl->session.typeExists(typeName, outResolvedName);
    }
    return m_impl->session.typeExists(typeName);
}

bool PdbReader::ListClassNames(std::vector<std::string>& outNames,
                               const SymbolProgressCallback& progress) {
    if (!m_initialized) return false;
    if (progress) progress(0, 0, "Building class name index...");
    return m_impl->session.listClassNames([&](std::string_view name) -> bool {
        outNames.emplace_back(name);
        return true;
    });
}

bool PdbReader::FindClassLayout(const std::string& className,
                                ClassLayoutInfo& outLayout) {
    if (!m_initialized) return false;

    ::ark::ClassLayout layout = m_impl->session.getClassLayout(className);
    if (!layout) return false;

    outLayout.className     = className;
    outLayout.totalSize     = layout.totalSize();
    outLayout.baseClassName = layout.baseClass();
    outLayout.members.clear();

    char nameBuf[ARK_PDB_NAME_BUF];
    char typeBuf[ARK_PDB_TYPE_BUF];
    const int32_t n = layout.memberCount();
    for (int32_t i = 0; i < n; i++) {
        layout.getMemberName(i, nameBuf, sizeof(nameBuf));
        layout.getMemberType(i, typeBuf, sizeof(typeBuf));
        ClassMemberInfo m;
        m.name     = nameBuf;
        m.typeName = typeBuf;
        m.offset   = layout.getMemberOffset(i);
        m.size     = layout.getMemberSize(i);
        outLayout.members.push_back(std::move(m));
    }
    return true;
}

bool PdbReader::FindClassFunctions(const std::string& className,
                                   std::vector<ClassFunctionInfo>& outFunctions) {
    if (!m_initialized) return false;

    ::ark::ClassFunctionList fns = m_impl->session.getClassFunctions(className);
    if (!fns) return false;

    outFunctions.clear();
    char nameBuf[ARK_PDB_NAME_BUF];
    char decBuf[ARK_PDB_DECORATED_BUF];
    char retBuf[ARK_PDB_TYPE_BUF];
    char pnameBuf[ARK_PDB_NAME_BUF];
    char ptypeBuf[ARK_PDB_TYPE_BUF];

    const int32_t n = fns.count();
    for (int32_t i = 0; i < n; i++) {
        fns.getName(i, nameBuf, sizeof(nameBuf));
        fns.getDecoratedName(i, decBuf, sizeof(decBuf));
        fns.getReturnType(i, retBuf, sizeof(retBuf));

        ClassFunctionInfo fn;
        fn.name          = nameBuf;
        fn.decoratedName = decBuf;
        fn.returnType    = retBuf;
        fn.isStatic      = fns.isStatic(i);
        fn.isVirtual     = fns.isVirtual(i);
        fn.isConst       = fns.isConst(i);

        const int32_t pc = fns.paramCount(i);
        for (int32_t p = 0; p < pc; p++) {
            fns.getParamName(i, p, pnameBuf, sizeof(pnameBuf));
            fns.getParamType(i, p, ptypeBuf, sizeof(ptypeBuf));
            FunctionParamInfo param;
            param.name     = pnameBuf;
            param.typeName = ptypeBuf;
            fn.parameters.push_back(std::move(param));
        }
        outFunctions.push_back(std::move(fn));
    }
    return true;
}

// FindSymbolRVA is not required for the SDK generation workflow.
// Return false; the caller handles the missing implementation gracefully.
bool PdbReader::FindSymbolRVA(const std::string&, uint64_t&) {
    return false;
}

// ListSymbols is not on the critical path for SDK generation.
// Return true with an empty list; the caller can fall back to the LLVM path
// or handle the missing data.
bool PdbReader::ListSymbols(std::vector<PdbSymbolSummary>&,
                            const SymbolListOptions&,
                            const SymbolProgressCallback&) {
    return true;
}

// ResolveTypeName / GetDecoratedName are private helpers used only in the DIA
// and LLVM backends; they are not part of the public PdbReader interface.
// pdb_reader_pdbrs.cpp does not implement them.

} // namespace ark

#endif // !_WIN32
```

---

## 4. Update CMakeLists.txt in ArkSdkGen

Replace the `else()` block in `CMakeLists.txt` (the Linux/LLVM section) with
an ArkPdbReader block:

```cmake
else()
    # ------------------------------------------------------------------ #
    # Linux: use ArkPdbReader (Rust/CodeView direct-parse backend)         #
    # instead of LLVM.                                                     #
    # ------------------------------------------------------------------ #
    set(ARKSDKGEN_PDB_READER_SRC src/native/pdb_reader_pdbrs.cpp)

    set(ARK_PDB_READER_DIR
        "${CMAKE_CURRENT_SOURCE_DIR}/../ArkPdbReader"
        CACHE PATH "Path to the ArkPdbReader source directory")

    add_subdirectory("${ARK_PDB_READER_DIR}" ark-pdb-reader-build)

    target_link_libraries(ark-sdk-gen-core    PRIVATE ark-pdb-reader)
    target_link_libraries(ark-sdk-gen         PRIVATE ark-pdb-reader)
    target_link_libraries(ark-sdk-gen-interop PRIVATE ark-pdb-reader)
endif()
```

Remove the `find_package(LLVM ...)` call and all LLVM `target_include_directories`,
`target_link_libraries`, and `target_compile_definitions` blocks from the Linux
path.

---

## 5. Update sdk-gen.proj (MSBuild wrapper)

In `sdk-gen.proj`, locate the Linux `<Exec>` target that runs CMake for
the native layer and add a Cargo build step before it:

```xml
<Target Name="BuildNativeLinux" Condition="'$(OS)' != 'Windows_NT'">
  <!-- Step 1: build ArkPdbReader Rust library -->
  <Exec Command="cargo build --release"
        WorkingDirectory="$(MSBuildThisFileDirectory)../ArkPdbReader" />

  <!-- Step 2: configure and build ArkSdkGen native layer as before -->
  <MakeDir Directories="$(NativeBuildDir)" />
  <Exec Command="cmake ... -DARK_PDB_READER_BUILD_RUST=OFF ..."
        WorkingDirectory="$(NativeBuildDir)" />
  <Exec Command="cmake --build . --config Release"
        WorkingDirectory="$(NativeBuildDir)" />
</Target>
```

Pass `-DARK_PDB_READER_BUILD_RUST=OFF` to CMake since the Cargo build has
already happened. CMake will only create the imported target pointing at the
pre-built `.so`.

---

## 6. Set the symbolBackend in local config

In ArkSdkGen's `local.json` (the `LocalConfig`), set:

```json
{
  "symbolBackend": "pdbrs"
}
```

If `engine_session.cpp` / `sdk_core.cpp` checks the `symbolBackend` field,
add a branch for `"pdbrs"` that selects the new backend. If the field is not
yet checked (the LLVM backend is the unconditional Linux default), no change
is needed — the new file simply replaces the old one via the CMake source list.

---

## 7. Verification checklist

After making the above changes, verify with:

```bash
# 1. Build ArkPdbReader
cd ArkPdbReader && cargo build --release

# 2. Build ArkSdkGen
cd ../ArkSdkGen
cmake -B build-linux/Release -DCMAKE_BUILD_TYPE=Release \
      -DARK_PDB_READER_DIR=../ArkPdbReader \
      -DARK_PDB_READER_BUILD_RUST=OFF
cmake --build build-linux/Release
dotnet build ArkSdkGen.sln -c Release

# 3. Run a smoke test
./build-linux/Release/ark-sdk-gen \
  --pdb /path/to/ShooterGame.pdb \
  --scan-classes

# Expected: output includes AGameModeBase, AActor, UObject etc.
# Expected: member and function counts > 0 for those classes.
# Expected: no LLVM .so dependency in ldd output.
```

---

## 8. Rollback

To revert to the LLVM backend, restore the original `CMakeLists.txt` `else()`
block and set `ARKSDKGEN_PDB_READER_SRC` back to
`src/native/pdb_reader_llvm.cpp`. No changes to any other file are needed.
