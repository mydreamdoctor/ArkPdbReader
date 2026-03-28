/**
 * ark_pdb_reader.hpp — C++ RAII wrapper for the ArkPdbReader C API.
 *
 * Provides three RAII types:
 *
 *   ark::PdbSession         — wraps ArkPdbSession*
 *   ark::ClassLayout        — wraps ArkLayoutHandle*
 *   ark::ClassFunctionList  — wraps ArkFunctionListHandle*
 *
 * And two value types for iterating results:
 *
 *   ark::MemberView         — non-owning view of one data member
 *   ark::FunctionView       — non-owning view of one member function
 *   ark::ParamView          — non-owning view of one function parameter
 *
 * ============================================================================
 * Usage example
 * ============================================================================
 *
 *   // Open the PDB.
 *   ark::PdbSession pdb("/path/to/ShooterGame.pdb");
 *   if (!pdb) { std::cerr << "open failed\n"; return 1; }
 *
 *   // Enumerate UE-style class names.
 *   pdb.listClassNames([](std::string_view name) {
 *       std::cout << name << "\n";
 *       return true; // false to stop early
 *   });
 *
 *   // Check type existence.
 *   std::string resolved;
 *   if (pdb.typeExists("AGameModeBase", &resolved))
 *       std::cout << "Resolved: " << resolved << "\n";
 *
 *   // Get class layout.
 *   ark::ClassLayout layout = pdb.getClassLayout("AGameModeBase");
 *   if (layout) {
 *       std::cout << "Base: " << layout.baseClass() << "\n";
 *       std::cout << "Size: " << layout.totalSize() << "\n";
 *       for (const auto& m : layout.members()) {
 *           std::cout << "  +" << m.offset << "  " << m.typeName
 *                     << "  " << m.name << "\n";
 *       }
 *   }
 *
 *   // Get member functions.
 *   // The first call may do a one-time lazy module-symbol scan to improve
 *   // parameter names. That work is cached and does not affect findSymbolRva().
 *   ark::ClassFunctionList fns = pdb.getClassFunctions("AGameModeBase");
 *   if (fns) {
 *       for (const auto& f : fns.functions()) {
 *           std::cout << f.returnType << " " << f.name << "(";
 *           bool first = true;
 *           for (const auto& p : f.params) {
 *               if (!first) std::cout << ", ";
 *               std::cout << p.typeName << " " << p.paramName;
 *               first = false;
 *           }
 *           std::cout << ")";
 *           if (f.isVirtual) std::cout << " [virtual]";
 *           if (f.isStatic)  std::cout << " [static]";
 *           if (f.isConst)   std::cout << " [const]";
 *           std::cout << "\n  // " << f.decoratedName << "\n";
 *       }
 *   }
 *
 * ============================================================================
 * Integration with ArkSdkGen
 * ============================================================================
 *
 * To replace the LLVM backend (pdb_reader_llvm.cpp) with this library:
 *
 *   1. Add ArkPdbReader as an add_subdirectory or find_package target.
 *   2. Create a new file src/native/pdb_reader_pdbrs.cpp implementing the
 *      existing `ark::PdbReader` class interface (from pdb_reader.h) using
 *      this wrapper.
 *   3. In CMakeLists.txt, replace the LLVM dependency block with
 *      `target_link_libraries(ark-sdk-gen-core PRIVATE ark-pdb-reader)`.
 *   4. Point ARK_PDB_READER_DIR at the ArkPdbReader build directory.
 *
 * The full integration contract is documented in docs/integration-guide.md.
 */

#pragma once

#include "ark_pdb_reader.h"

#include <array>
#include <cstring>
#include <functional>
#include <string>
#include <string_view>
#include <vector>

namespace ark {

// ============================================================================
// Value types
// ============================================================================

enum class TypeKind {
    Class = ARK_PDB_TYPE_KIND_CLASS,
    Struct = ARK_PDB_TYPE_KIND_STRUCT
};

enum class SymbolKind {
    Class = ARK_PDB_SYMBOL_KIND_CLASS,
    Struct = ARK_PDB_SYMBOL_KIND_STRUCT,
    Union = ARK_PDB_SYMBOL_KIND_UNION,
    Enum = ARK_PDB_SYMBOL_KIND_ENUM,
    GlobalFunction = ARK_PDB_SYMBOL_KIND_GLOBAL_FUNCTION,
    GlobalSymbol = ARK_PDB_SYMBOL_KIND_GLOBAL_SYMBOL
};

struct TypeEntryView {
    std::string name;
    TypeKind kind = TypeKind::Class;
};

struct SymbolEntryView {
    std::string name;
    SymbolKind kind = SymbolKind::Class;
};

/** One data member of a class. */
struct MemberView {
    std::string name;
    std::string typeName;
    int32_t     offset = 0;
    uint32_t    size   = 0;
};

/** One parameter of a member function. */
struct ParamView {
    std::string paramName;
    std::string typeName;
};

/** One member function of a class. */
struct FunctionView {
    std::string           name;
    std::string           decoratedName;
    std::string           returnType;
    std::vector<ParamView> params;
    bool isStatic  = false;
    bool isVirtual = false;
    bool isConst   = false;
};

// ============================================================================
// ClassLayout
// ============================================================================

/**
 * RAII wrapper for ArkLayoutHandle.
 *
 * Move-only (copying a layout result is explicit via copyLayout()).
 */
class ClassLayout {
public:
    ClassLayout() noexcept = default;

    explicit ClassLayout(ArkLayoutHandle* handle) noexcept
        : handle_(handle) {}

    ClassLayout(ClassLayout&& other) noexcept
        : handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    ClassLayout& operator=(ClassLayout&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~ClassLayout() { reset(); }

    ClassLayout(const ClassLayout&)            = delete;
    ClassLayout& operator=(const ClassLayout&) = delete;

    /** True if the layout was found (handle is non-null). */
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    /** Direct base class name, or empty string if none. */
    std::string baseClass() const {
        std::array<char, ARK_PDB_NAME_BUF> buf{};
        ark_pdb_layout_get_base_class(handle_, buf.data(), buf.size());
        return buf.data();
    }

    /** Total size of the struct/class in bytes. */
    uint32_t totalSize() const {
        return ark_pdb_layout_get_total_size(handle_);
    }

    /** Number of data members. */
    int32_t memberCount() const {
        return ark_pdb_layout_get_member_count(handle_);
    }

    /**
     * Return all data members as a vector of MemberView.
     *
     * Allocates once; O(N) in member count.  For hot loops prefer the
     * raw index-based accessors directly.
     */
    std::vector<MemberView> members() const {
        const int32_t n = memberCount();
        std::vector<MemberView> result;
        result.reserve(static_cast<size_t>(n));

        std::array<char, ARK_PDB_NAME_BUF> nameBuf{};
        std::array<char, ARK_PDB_TYPE_BUF> typeBuf{};

        for (int32_t i = 0; i < n; ++i) {
            ark_pdb_layout_get_member_name(handle_, i, nameBuf.data(), nameBuf.size());
            ark_pdb_layout_get_member_type(handle_, i, typeBuf.data(), typeBuf.size());
            result.push_back({
                nameBuf.data(),
                typeBuf.data(),
                ark_pdb_layout_get_member_offset(handle_, i),
                ark_pdb_layout_get_member_size(handle_, i),
            });
        }
        return result;
    }

    // -- Raw index-based accessors (avoid allocation) ----------------------

    void getMemberName(int32_t i, char* buf, size_t len) const {
        ark_pdb_layout_get_member_name(handle_, i, buf, len);
    }
    void getMemberType(int32_t i, char* buf, size_t len) const {
        ark_pdb_layout_get_member_type(handle_, i, buf, len);
    }
    int32_t  getMemberOffset(int32_t i) const {
        return ark_pdb_layout_get_member_offset(handle_, i);
    }
    uint32_t getMemberSize(int32_t i) const {
        return ark_pdb_layout_get_member_size(handle_, i);
    }

private:
    void reset() {
        if (handle_) {
            ark_pdb_layout_free(handle_);
            handle_ = nullptr;
        }
    }

    ArkLayoutHandle* handle_ = nullptr;
};

// ============================================================================
// ClassFunctionList
// ============================================================================

/**
 * RAII wrapper for ArkFunctionListHandle.
 * Move-only.
 */
class ClassFunctionList {
public:
    ClassFunctionList() noexcept = default;

    explicit ClassFunctionList(ArkFunctionListHandle* handle) noexcept
        : handle_(handle) {}

    ClassFunctionList(ClassFunctionList&& other) noexcept
        : handle_(other.handle_) {
        other.handle_ = nullptr;
    }

    ClassFunctionList& operator=(ClassFunctionList&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    ~ClassFunctionList() { reset(); }

    ClassFunctionList(const ClassFunctionList&)            = delete;
    ClassFunctionList& operator=(const ClassFunctionList&) = delete;

    /** True if the function list was found. */
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    /** Number of functions in the list. */
    int32_t count() const {
        return ark_pdb_funclist_get_count(handle_);
    }

    /**
     * Return all functions as a vector of FunctionView.
     *
     * Allocates; O(F * P) in function and parameter counts.
     */
    std::vector<FunctionView> functions() const {
        const int32_t n = count();
        std::vector<FunctionView> result;
        result.reserve(static_cast<size_t>(n));

        std::array<char, ARK_PDB_NAME_BUF>      nameBuf{};
        std::array<char, ARK_PDB_DECORATED_BUF> decBuf{};
        std::array<char, ARK_PDB_TYPE_BUF>      typeBuf{};
        std::array<char, ARK_PDB_NAME_BUF>      pnameBuf{};
        std::array<char, ARK_PDB_TYPE_BUF>      ptypeBuf{};

        for (int32_t i = 0; i < n; ++i) {
            ark_pdb_funclist_get_name(handle_, i, nameBuf.data(), nameBuf.size());
            ark_pdb_funclist_get_decorated_name(handle_, i, decBuf.data(), decBuf.size());
            ark_pdb_funclist_get_return_type(handle_, i, typeBuf.data(), typeBuf.size());

            FunctionView fn;
            fn.name          = nameBuf.data();
            fn.decoratedName = decBuf.data();
            fn.returnType    = typeBuf.data();
            fn.isStatic      = ark_pdb_funclist_is_static(handle_, i);
            fn.isVirtual     = ark_pdb_funclist_is_virtual(handle_, i);
            fn.isConst       = ark_pdb_funclist_is_const(handle_, i);

            const int32_t pc = ark_pdb_funclist_get_param_count(handle_, i);
            fn.params.reserve(static_cast<size_t>(pc));
            for (int32_t p = 0; p < pc; ++p) {
                ark_pdb_funclist_get_param_name(handle_, i, p, pnameBuf.data(), pnameBuf.size());
                ark_pdb_funclist_get_param_type(handle_, i, p, ptypeBuf.data(), ptypeBuf.size());
                fn.params.push_back({ pnameBuf.data(), ptypeBuf.data() });
            }

            result.push_back(std::move(fn));
        }
        return result;
    }

    // -- Raw index-based accessors -----------------------------------------

    void getName(int32_t i, char* buf, size_t len) const {
        ark_pdb_funclist_get_name(handle_, i, buf, len);
    }
    void getDecoratedName(int32_t i, char* buf, size_t len) const {
        ark_pdb_funclist_get_decorated_name(handle_, i, buf, len);
    }
    void getReturnType(int32_t i, char* buf, size_t len) const {
        ark_pdb_funclist_get_return_type(handle_, i, buf, len);
    }
    bool isStatic(int32_t i)  const { return ark_pdb_funclist_is_static(handle_, i); }
    bool isVirtual(int32_t i) const { return ark_pdb_funclist_is_virtual(handle_, i); }
    bool isConst(int32_t i)   const { return ark_pdb_funclist_is_const(handle_, i); }
    int32_t paramCount(int32_t fi) const {
        return ark_pdb_funclist_get_param_count(handle_, fi);
    }
    void getParamName(int32_t fi, int32_t pi, char* buf, size_t len) const {
        ark_pdb_funclist_get_param_name(handle_, fi, pi, buf, len);
    }
    void getParamType(int32_t fi, int32_t pi, char* buf, size_t len) const {
        ark_pdb_funclist_get_param_type(handle_, fi, pi, buf, len);
    }

private:
    void reset() {
        if (handle_) {
            ark_pdb_funclist_free(handle_);
            handle_ = nullptr;
        }
    }

    ArkFunctionListHandle* handle_ = nullptr;
};

// ============================================================================
// PdbSession
// ============================================================================

/**
 * RAII wrapper for ArkPdbSession.
 * Move-only.
 */
class PdbSession {
public:
    PdbSession() noexcept = default;

    /** Open a PDB file.  Check operator bool() before using. */
    explicit PdbSession(const char* path) noexcept
        : session_(ark_pdb_open(path)) {}

    explicit PdbSession(const std::string& path) noexcept
        : PdbSession(path.c_str()) {}

    PdbSession(PdbSession&& other) noexcept
        : session_(other.session_) {
        other.session_ = nullptr;
    }

    PdbSession& operator=(PdbSession&& other) noexcept {
        if (this != &other) {
            reset();
            session_ = other.session_;
            other.session_ = nullptr;
        }
        return *this;
    }

    ~PdbSession() { reset(); }

    PdbSession(const PdbSession&)            = delete;
    PdbSession& operator=(const PdbSession&) = delete;

    /** True if the session was opened successfully. */
    explicit operator bool() const noexcept { return session_ != nullptr; }

    /** Retrieve the last error string (empty if no error). */
    const char* lastError() const noexcept {
        return ark_pdb_last_error(session_);
    }

    // -- Class name enumeration --------------------------------------------

    /**
     * Enumerate Unreal Engine–style type entries with cached class/struct kind.
     *
     * @param callback  Called with each entry (name + kind) in sorted order.
     *                  Return true to continue, false to stop.
     * @return          true on success, false on error.
     */
    template <typename Fn>
    bool listTypeEntries(Fn&& callback) const {
        struct Ctx {
            Fn* fn;
        } ctx{ &callback };

        return ark_pdb_list_type_entries(
            session_,
            [](const char* name, ArkPdbTypeKind kind, void* ud) -> bool {
                auto* c = static_cast<Ctx*>(ud);
                return (*c->fn)(TypeEntryView{
                    std::string(name),
                    kind == ARK_PDB_TYPE_KIND_STRUCT ? TypeKind::Struct : TypeKind::Class
                });
            },
            &ctx);
    }

    /**
     * Enumerate Unreal Engine–style class names.
     *
     * @param callback  Called with each name (std::string_view) in sorted
     *                  order.  Return true to continue, false to stop.
     * @return          true on success, false on error.
     */
    template <typename Fn>
    bool listClassNames(Fn&& callback) const {
        return listTypeEntries([&](const TypeEntryView& entry) {
            return callback(std::string_view(entry.name));
        });
    }

    /**
     * Enumerate display-ready symbol entries.
     *
     * @param includeGlobalFunctions  Include global function names from IPI.
     * @param includePublicSymbols    Include public symbol names from PSI/GSS.
     * @param callback                Called with each entry in sorted order.
     *                                Return true to continue, false to stop.
     * @return                        true on success, false on error.
     */
    template <typename Fn>
    bool listSymbolEntries(
        bool includeGlobalFunctions,
        bool includePublicSymbols,
        Fn&& callback) const {
        struct Ctx {
            Fn* fn;
        } ctx{ &callback };

        return ark_pdb_list_symbol_entries(
            session_,
            includeGlobalFunctions,
            includePublicSymbols,
            [](const char* name, ArkPdbSymbolKind kind, void* ud) -> bool {
                auto* c = static_cast<Ctx*>(ud);
                return (*c->fn)(SymbolEntryView{
                    std::string(name),
                    static_cast<SymbolKind>(kind)
                });
            },
            &ctx);
    }

    // -- Type existence ----------------------------------------------------

    /**
     * Check whether a named type exists in the PDB (case-insensitive).
     *
     * @param name      Type name to search for.
     * @param resolved  If non-null, receives the canonical name on success.
     * @return          true if found.
     */
    bool typeExists(const char* name, std::string* resolved = nullptr) const {
        char buf[ARK_PDB_NAME_BUF] = {};
        const bool found = ark_pdb_type_exists(
            session_, name,
            resolved ? buf : nullptr,
            resolved ? sizeof(buf) : 0);
        if (found && resolved)
            *resolved = buf;
        return found;
    }

    bool typeExists(const std::string& name, std::string* resolved = nullptr) const {
        return typeExists(name.c_str(), resolved);
    }

    // -- Symbol lookup -----------------------------------------------------

    bool findSymbolRva(const char* decoratedName, uint64_t* outRva = nullptr) const {
        uint64_t localRva = 0;
        const bool found = ark_pdb_find_symbol_rva(
            session_,
            decoratedName,
            outRva ? outRva : &localRva);
        return found;
    }

    bool findSymbolRva(const std::string& decoratedName, uint64_t* outRva = nullptr) const {
        return findSymbolRva(decoratedName.c_str(), outRva);
    }

    // -- Class layout ------------------------------------------------------

    /**
     * Find the data member layout of a named class or struct.
     *
     * @return  ClassLayout RAII wrapper (check operator bool()).
     */
    ClassLayout getClassLayout(const char* className) const {
        return ClassLayout(ark_pdb_find_class_layout(session_, className));
    }

    ClassLayout getClassLayout(const std::string& className) const {
        return getClassLayout(className.c_str());
    }

    // -- Member functions --------------------------------------------------

    /**
     * Find all member functions of a named class or struct.
     *
     * The first call may trigger a one-time lazy module-symbol scan to recover
     * better parameter names. That cache does not affect open-time symbol RVA lookup.
     * Overload matching on that path also normalizes common type spelling
     * differences before it gives up on recovering real names.
     *
     * @return  ClassFunctionList RAII wrapper (check operator bool()).
     */
    ClassFunctionList getClassFunctions(const char* className) const {
        return ClassFunctionList(ark_pdb_find_class_functions(session_, className));
    }

    ClassFunctionList getClassFunctions(const std::string& className) const {
        return getClassFunctions(className.c_str());
    }

private:
    void reset() {
        if (session_) {
            ark_pdb_close(session_);
            session_ = nullptr;
        }
    }

    ArkPdbSession* session_ = nullptr;
};

} // namespace ark
