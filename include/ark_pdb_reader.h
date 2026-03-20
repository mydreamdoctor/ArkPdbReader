/**
 * ark_pdb_reader.h — C API for ArkPdbReader
 *
 * High-performance PDB reader for ARK: Survival Ascended dedicated server PDB
 * files. Extracts class/struct names, data member layouts, and member function
 * signatures directly from CodeView TPI records.
 *
 * ============================================================================
 * Quick example (C)
 * ============================================================================
 *
 *   ArkPdbSession* pdb = ark_pdb_open("/path/to/ShooterGame.pdb");
 *   if (!pdb) { fprintf(stderr, "open failed\n"); return 1; }
 *
 *   // -- List class names --------------------------------------------------
 *   ark_pdb_list_class_names(pdb, [](const char* name, void*) -> bool {
 *       printf("%s\n", name); return true;
 *   }, NULL);
 *
 *   // -- Get class layout --------------------------------------------------
 *   ArkLayoutHandle* layout = ark_pdb_find_class_layout(pdb, "AGameModeBase");
 *   if (layout) {
 *       char base[256]; ark_pdb_layout_get_base_class(layout, base, sizeof(base));
 *       int n = ark_pdb_layout_get_member_count(layout);
 *       for (int i = 0; i < n; i++) {
 *           char name[256], type[512];
 *           ark_pdb_layout_get_member_name(layout, i, name, sizeof(name));
 *           ark_pdb_layout_get_member_type(layout, i, type, sizeof(type));
 *           int32_t off = ark_pdb_layout_get_member_offset(layout, i);
 *           printf("  +%d  %s  %s\n", off, type, name);
 *       }
 *       ark_pdb_layout_free(layout);
 *   }
 *
 *   // -- Get member functions ----------------------------------------------
 *   ArkFunctionListHandle* fns = ark_pdb_find_class_functions(pdb, "AGameModeBase");
 *   if (fns) {
 *       int n = ark_pdb_funclist_get_count(fns);
 *       for (int i = 0; i < n; i++) {
 *           char name[256], ret[512], decorated[1024];
 *           ark_pdb_funclist_get_name(fns, i, name, sizeof(name));
 *           ark_pdb_funclist_get_return_type(fns, i, ret, sizeof(ret));
 *           ark_pdb_funclist_get_decorated_name(fns, i, decorated, sizeof(decorated));
 *           printf("  %s %s(", ret, name);
 *           int pc = ark_pdb_funclist_get_param_count(fns, i);
 *           for (int p = 0; p < pc; p++) {
 *               char pname[128], ptype[512];
 *               ark_pdb_funclist_get_param_name(fns, i, p, pname, sizeof(pname));
 *               ark_pdb_funclist_get_param_type(fns, i, p, ptype, sizeof(ptype));
 *               if (p > 0) printf(", ");
 *               printf("%s %s", ptype, pname);
 *           }
 *           printf(")  // %s\n", decorated);
 *       }
 *       ark_pdb_funclist_free(fns);
 *   }
 *
 *   ark_pdb_close(pdb);
 *
 * ============================================================================
 * Threading
 * ============================================================================
 *
 * A single session is NOT thread-safe. Do not call any function on the same
 * session from multiple threads concurrently. Create one session per thread
 * if concurrent access is needed.
 *
 * ============================================================================
 * Buffer size recommendations
 * ============================================================================
 *
 * All string-output functions write into caller-supplied buffers and
 * null-terminate (truncating if needed).  Recommended buffer sizes:
 *
 *   Class / function names:  256 bytes  (ARK_PDB_NAME_BUF)
 *   Type names:              512 bytes  (ARK_PDB_TYPE_BUF)
 *   Decorated names:        1024 bytes  (ARK_PDB_DECORATED_BUF)
 *   Base class names:        256 bytes  (ARK_PDB_NAME_BUF)
 */

#ifndef ARK_PDB_READER_H
#define ARK_PDB_READER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -------------------------------------------------------------------------- */
/* Recommended buffer sizes                                                   */
/* -------------------------------------------------------------------------- */

#define ARK_PDB_NAME_BUF      256
#define ARK_PDB_TYPE_BUF      512
#define ARK_PDB_DECORATED_BUF 1024

/* -------------------------------------------------------------------------- */
/* Opaque handle types                                                        */
/* -------------------------------------------------------------------------- */

/**
 * An open PDB session.  Created by ark_pdb_open, destroyed by ark_pdb_close.
 */
typedef struct ArkPdbSession ArkPdbSession;

/**
 * Class layout result.  Created by ark_pdb_find_class_layout,
 * destroyed by ark_pdb_layout_free.  May outlive the session.
 */
typedef struct ArkLayoutHandle ArkLayoutHandle;

/**
 * Function list result.  Created by ark_pdb_find_class_functions,
 * destroyed by ark_pdb_funclist_free.  May outlive the session.
 */
typedef struct ArkFunctionListHandle ArkFunctionListHandle;

/* -------------------------------------------------------------------------- */
/* Lightweight type enumeration                                               */
/* -------------------------------------------------------------------------- */

typedef enum ArkPdbTypeKind {
    ARK_PDB_TYPE_KIND_CLASS = 1,
    ARK_PDB_TYPE_KIND_STRUCT = 2
} ArkPdbTypeKind;

/* -------------------------------------------------------------------------- */
/* Session lifecycle                                                          */
/* -------------------------------------------------------------------------- */

/**
 * Open a PDB file.
 *
 * Reads the TPI stream and Global Symbol Stream into memory.
 * This is the only I/O-heavy operation; all subsequent queries are in-memory.
 *
 * @param path  UTF-8 null-terminated path to the .pdb file.
 * @return      Session handle on success, NULL on failure.
 *              Failure details are printed to stderr.
 */
ArkPdbSession* ark_pdb_open(const char* path);

/**
 * Close a session and free all associated memory.
 *
 * Any layout or function-list handles that were created from this session
 * remain valid after this call (they own their data independently).
 *
 * @param session  May be NULL (no-op).
 */
void ark_pdb_close(ArkPdbSession* session);

/**
 * Retrieve the last error message stored on the session.
 *
 * Returns a pointer valid until the next mutable call on this session or
 * until ark_pdb_close.  Do NOT free the returned pointer.
 *
 * @param session  May be NULL (returns a pointer to an empty string).
 * @return         Null-terminated UTF-8 error string (never NULL itself).
 */
const char* ark_pdb_last_error(const ArkPdbSession* session);

/* -------------------------------------------------------------------------- */
/* Class name enumeration                                                     */
/* -------------------------------------------------------------------------- */

/**
 * Callback type for ark_pdb_list_class_names.
 *
 * @param name       Null-terminated UTF-8 class name.
 * @param user_data  The pointer passed to ark_pdb_list_class_names.
 * @return           true to continue enumeration, false to stop early.
 */
typedef bool (*ArkClassNameCallback)(const char* name, void* user_data);

/**
 * Callback type for ark_pdb_list_type_entries.
 *
 * @param name       Null-terminated UTF-8 type name.
 * @param kind       Lightweight kind captured from the UDT record.
 * @param user_data  The pointer passed to ark_pdb_list_type_entries.
 * @return           true to continue enumeration, false to stop early.
 */
typedef bool (*ArkTypeEntryCallback)(const char* name, ArkPdbTypeKind kind, void* user_data);

/**
 * Enumerate all Unreal Engine–style class and struct names from the PDB.
 *
 * Only names matching the UE top-level class pattern are included:
 *   [A|U|F|E|T|I] followed by an uppercase letter, no templates or namespaces.
 *
 * Names are delivered in sorted (alphabetical) order.
 *
 * Building the name index takes O(N) in the number of TPI records on the
 * first call; subsequent calls use the cached index.
 *
 * @param session    Open session handle.
 * @param callback   Called once per class name.
 * @param user_data  Forwarded to every callback invocation.
 * @return           true on success, false on error.
 */
bool ark_pdb_list_class_names(
    ArkPdbSession*      session,
    ArkClassNameCallback callback,
    void*               user_data);

/**
 * Enumerate all Unreal Engine–style class and struct names with kind.
 *
 * This reuses the cached TPI name index built on first use. The reported kind
 * is captured during that same one-time index build, so enumeration does not
 * trigger a second full pass over the PDB.
 *
 * @param session    Open session handle.
 * @param callback   Called once per type entry.
 * @param user_data  Forwarded to every callback invocation.
 * @return           true on success, false on error.
 */
bool ark_pdb_list_type_entries(
    ArkPdbSession*       session,
    ArkTypeEntryCallback callback,
    void*                user_data);

/* -------------------------------------------------------------------------- */
/* Symbol RVA lookup                                                          */
/* -------------------------------------------------------------------------- */

/**
 * Look up the RVA (Relative Virtual Address) of a public symbol by its
 * decorated (mangled) name.
 *
 * The decorated name must match exactly, including the leading '?' for C++
 * symbols, e.g. "?GUObjectArray@@3VFUObjectArray@@A".
 *
 * Building the symbol index takes O(N) in the number of public symbols on
 * the first call; subsequent calls reuse the cached index.
 *
 * @param session        Open session handle.
 * @param decorated_name Exact decorated name (null-terminated UTF-8).
 * @param out_rva        Receives the 64-bit RVA on success. May be NULL.
 * @return               true if found and out_rva written; false otherwise.
 */
bool ark_pdb_find_symbol_rva(
    ArkPdbSession* session,
    const char*    decorated_name,
    uint64_t*      out_rva);

/* -------------------------------------------------------------------------- */
/* Type existence check                                                       */
/* -------------------------------------------------------------------------- */

/**
 * Check whether a named type exists in the PDB (case-insensitive lookup).
 *
 * @param session            Open session handle.
 * @param name               Type name to search for (null-terminated UTF-8).
 * @param out_resolved_name  If non-NULL and the type is found, receives the
 *                           canonical (exact-case) name stored in the PDB.
 *                           Null-terminated, truncated to buf_len if needed.
 * @param buf_len            Size of out_resolved_name in bytes.
 * @return                   true if found, false if not found or on error.
 */
bool ark_pdb_type_exists(
    ArkPdbSession* session,
    const char*    name,
    char*          out_resolved_name,
    size_t         buf_len);

/* -------------------------------------------------------------------------- */
/* Class layout                                                               */
/* -------------------------------------------------------------------------- */

/**
 * Find the data member layout of a class or struct.
 *
 * @param session     Open session handle.
 * @param class_name  Class or struct name (case-insensitive, null-terminated).
 * @return            Layout handle on success, NULL if not found or on error.
 *                    Must be freed with ark_pdb_layout_free.
 */
ArkLayoutHandle* ark_pdb_find_class_layout(
    ArkPdbSession* session,
    const char*    class_name);

/**
 * Free a layout handle.  NULL is safe (no-op).
 */
void ark_pdb_layout_free(ArkLayoutHandle* handle);

/**
 * Write the direct base class name into buf (empty string if none).
 *
 * @param handle   Layout handle from ark_pdb_find_class_layout.
 * @param buf      Caller-allocated output buffer.
 * @param buf_len  Size of buf in bytes.
 */
void ark_pdb_layout_get_base_class(
    const ArkLayoutHandle* handle,
    char*                  buf,
    size_t                 buf_len);

/**
 * Return the total size of the class/struct in bytes.
 */
uint32_t ark_pdb_layout_get_total_size(const ArkLayoutHandle* handle);

/**
 * Return the number of data members in the layout.
 */
int32_t ark_pdb_layout_get_member_count(const ArkLayoutHandle* handle);

/**
 * Write the field name of the member at index into buf.
 *
 * @param handle   Layout handle.
 * @param index    Zero-based member index (0 .. member_count - 1).
 * @param buf      Caller-allocated output buffer.
 * @param buf_len  Size of buf in bytes.
 */
void ark_pdb_layout_get_member_name(
    const ArkLayoutHandle* handle,
    int32_t                index,
    char*                  buf,
    size_t                 buf_len);

/**
 * Write the C++ type name of the member at index into buf.
 */
void ark_pdb_layout_get_member_type(
    const ArkLayoutHandle* handle,
    int32_t                index,
    char*                  buf,
    size_t                 buf_len);

/**
 * Return the byte offset of the member at index from the start of the struct.
 */
int32_t ark_pdb_layout_get_member_offset(
    const ArkLayoutHandle* handle,
    int32_t                index);

/**
 * Return the size in bytes of the member at index (0 = unknown).
 */
uint32_t ark_pdb_layout_get_member_size(
    const ArkLayoutHandle* handle,
    int32_t                index);

/* -------------------------------------------------------------------------- */
/* Member functions                                                           */
/* -------------------------------------------------------------------------- */

/**
 * Find all member functions of a class or struct.
 *
 * Excludes constructors, destructors, and operator overloads.
 *
 * Building the symbol index (for decorated-name resolution) takes O(N) in
 * the number of public symbols on the first call.
 *
 * @param session     Open session handle.
 * @param class_name  Class or struct name (case-insensitive, null-terminated).
 * @return            Function list handle on success, NULL on error.
 *                    An empty list (0 functions) still returns a non-NULL handle.
 *                    Must be freed with ark_pdb_funclist_free.
 */
ArkFunctionListHandle* ark_pdb_find_class_functions(
    ArkPdbSession* session,
    const char*    class_name);

/**
 * Free a function list handle.  NULL is safe (no-op).
 */
void ark_pdb_funclist_free(ArkFunctionListHandle* handle);

/**
 * Return the number of functions in the list.
 */
int32_t ark_pdb_funclist_get_count(const ArkFunctionListHandle* handle);

/**
 * Write the short name of function at func_index into buf.
 * Example: "GetPlayerName"
 */
void ark_pdb_funclist_get_name(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index,
    char*                        buf,
    size_t                       buf_len);

/**
 * Write the MSVC-mangled decorated name of function at func_index into buf.
 * Example: "?GetPlayerName@APlayerController@@QEAA..."
 * Writes an empty string if the symbol was not found in the public symbol table.
 */
void ark_pdb_funclist_get_decorated_name(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index,
    char*                        buf,
    size_t                       buf_len);

/**
 * Write the C++ return type of function at func_index into buf.
 * Example: "void", "FString*", "bool"
 */
void ark_pdb_funclist_get_return_type(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index,
    char*                        buf,
    size_t                       buf_len);

/** Return true if the function at func_index is static. */
bool ark_pdb_funclist_is_static(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index);

/** Return true if the function at func_index is virtual or pure virtual. */
bool ark_pdb_funclist_is_virtual(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index);

/** Return true if the function at func_index is const-qualified. */
bool ark_pdb_funclist_is_const(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index);

/**
 * Return the number of parameters of function at func_index.
 * Does not count the implicit `this` parameter.
 */
int32_t ark_pdb_funclist_get_param_count(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index);

/**
 * Write the name of parameter param_index of function func_index into buf.
 * PDB parameter names are often absent; "paramN" is used as a fallback.
 */
void ark_pdb_funclist_get_param_name(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index,
    int32_t                      param_index,
    char*                        buf,
    size_t                       buf_len);

/**
 * Write the C++ type name of parameter param_index of function func_index
 * into buf.
 */
void ark_pdb_funclist_get_param_type(
    const ArkFunctionListHandle* handle,
    int32_t                      func_index,
    int32_t                      param_index,
    char*                        buf,
    size_t                       buf_len);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* ARK_PDB_READER_H */
