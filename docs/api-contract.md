# ArkPdbReader — API Contract

This document defines the exact behaviour contract for `ArkPdbReader`. It is
the authoritative reference for anyone integrating this library into their
project.

---

## 1. Opening and closing a session

```c
ArkPdbSession* ark_pdb_open(const char* path);
void           ark_pdb_close(ArkPdbSession* session);
const char*    ark_pdb_last_error(const ArkPdbSession* session);
```

### Guarantees

- `ark_pdb_open` reads the **TPI stream** and **Global Symbol Stream** entirely
  into memory before returning. The underlying file handle is kept open but is
  not needed for any subsequent query.
- On failure `ark_pdb_open` returns `NULL`; the reason is printed to `stderr`.
- `ark_pdb_close(NULL)` is safe (no-op).
- Result handles (`ArkLayoutHandle*`, `ArkFunctionListHandle*`) may outlive
  the session. They do not hold a reference back to it.

---

## 2. Name index (class enumeration and type existence)

Both `ark_pdb_list_class_names` and `ark_pdb_type_exists` rely on the **name
index**, which is built lazily on the first call.

### Build cost

One sequential pass over all TPI records. For the ASA dedicated server PDB
(~800 k–2 M type records) expect roughly 1–3 seconds on first call. Subsequent
calls are O(1) hash lookups.

### Filter applied by `ark_pdb_list_class_names`

Only names passing the Unreal Engine top-level class pattern are returned:

- First character is one of: `A`, `U`, `F`, `E`, `T`, `I`
- Second character is an ASCII uppercase letter
- Name contains no `<` (no templates) and no `::` (no nested types)

This matches the filter in `pdb_reader_llvm.cpp`'s `IsPreferredTopLevelClassName`.

### Case sensitivity

All lookups are case-insensitive. The canonical (exact-case) name from the PDB
is always returned in output buffers.

### Forward references

The PDB may contain both a **forward declaration** (no field list) and a **full
definition** of the same class. The name index always stores the full
definition's TypeIndex. If only a forward declaration is present the class will
appear in the index but `find_class_layout` will return null.

---

## 3. Class layout

```c
ArkLayoutHandle* ark_pdb_find_class_layout(ArkPdbSession*, const char*);
void             ark_pdb_layout_free(ArkLayoutHandle*);
// ... accessors ...
```

### What is returned

| Field | Source in PDB |
|---|---|
| `base_class` | First `LF_BCLASS` field entry in the field list |
| `total_size` | `LF_CLASS.size` field (variable-length Number) |
| Members | All `LF_MEMBER` field entries (instance data members only) |
| Member offset | `LF_MEMBER.offset` |
| Member type name | Recursively resolved from `LF_MEMBER.ty` TypeIndex |
| Member size | Resolved from the member's type record (0 = unknown) |

### What is NOT returned

- Static data members (`LF_STMEM` field entries) — static members have no
  instance offset.
- Bit-field details — the type name includes the underlying integer type but
  bit width and position are not exposed.
- VTable pointer entries — these appear in some PDB layouts as unnamed members
  at offset 0.  They are included as regular members (name will be empty or
  `__vfptr`).

### Caching

Results are memoised in the session by exact-case class name. The second call
for the same class name returns the cached result.

---

## 4. Member functions

```c
ArkFunctionListHandle* ark_pdb_find_class_functions(ArkPdbSession*, const char*);
void                   ark_pdb_funclist_free(ArkFunctionListHandle*);
// ... accessors ...
```

### What is returned

| Field | Source in PDB |
|---|---|
| `name` | Short function name from `LF_ONEMETHOD.name` / `LF_METHOD.name` |
| `decorated_name` | Matched from the public symbol table (PSI) by class+method name |
| `return_type` | `LF_MFUNCTION.return_value` → type name resolved |
| `params` | `LF_MFUNCTION.arg_list` → `LF_ARGLIST` → each TypeIndex resolved |
| `is_static` | `LF_ONEMETHOD.attr` method-property bits == static (2) |
| `is_virtual` | method-property bits == virtual (1), intro-virtual (4), pure (5,6) |
| `is_const` | `LF_MFUNCTION.this` pointer's pointee has `LF_MODIFIER` const bit |

### Excluded

- Constructors (same name as class)
- Destructors (name starts with `~`)
- Operator overloads (name starts with `operator`)
- Methods with no RVA (compiler-generated, pure virtuals with no body)

### Decorated name resolution

The decorated name is resolved by matching the class name and method name
against MSVC-mangled public symbols in the GSS. The matching is a lightweight
parse of the `?MethodName@ClassName@@...` pattern — **no full demangler is
used**.

When multiple overloads exist (same class + method name), all decorated names
are stored. The `get_decorated_name` accessor returns the **first** match.
This may be incorrect for overloaded methods; the generator is expected to
disambiguate using the full signature when needed.

### Symbol index build cost

One sequential pass over the Global Symbol Stream on the first
`find_class_functions` call. Subsequent calls use the cached index.

---

## 5. Type name resolution

Type names are resolved recursively from TPI TypeIndex values to C++ strings.

| TypeIndex value | Resolution |
|---|---|
| < `type_index_begin` | Primitive (see table below) |
| → `LF_CLASS` / `LF_STRUCTURE` | Class name from record |
| → `LF_ENUM` | Enum name from record |
| → `LF_POINTER` | Pointee name + `*` (or `&` / `&&` for references) |
| → `LF_MODIFIER` | Underlying type (const/volatile stripped) |
| → `LF_ARRAY` | Element type + `[]` |
| → `LF_MFUNCTION` / `LF_PROCEDURE` | `void(*)()` placeholder |
| → `LF_BITFIELD` | Underlying integer type |
| → other | `"unknown"` |

Recursion depth is limited to 12. Types nested deeper return `"..."`.

### Primitive type table (subset)

| TypeIndex range | C++ name |
|---|---|
| `0x0003` | `void` |
| `0x0010` | `char` |
| `0x0020` | `unsigned char` |
| `0x0030` | `bool` |
| `0x0011` | `short` |
| `0x0021` | `unsigned short` |
| `0x0012`, `0x0074` | `int` |
| `0x0022`, `0x0075` | `unsigned int` |
| `0x0013`, `0x0076` | `int64_t` |
| `0x0023`, `0x0077` | `uint64_t` |
| `0x0040` | `float` |
| `0x0041` | `double` |
| Mode 6 (near 64-bit ptr) | base type + `*` |

---

## 6. Buffer sizes

All string-output functions accept a caller-supplied buffer and its size.
Strings are null-terminated and truncated (not overrun) if the buffer is too
small. The following sizes are sufficient for all known ASA PDB names:

| Macro | Value | Use for |
|---|---|---|
| `ARK_PDB_NAME_BUF` | 256 | Class names, method names, field names |
| `ARK_PDB_TYPE_BUF` | 512 | Type names (may be long for templates) |
| `ARK_PDB_DECORATED_BUF` | 1024 | Decorated C++ names |

---

## 7. Error handling

- All functions return `false` / `NULL` on error.
- Errors from session-level operations are stored on the session and retrievable
  via `ark_pdb_last_error`.
- Internal Rust panics are caught at the FFI boundary; the call returns
  `false` / `NULL` and a message is printed to `stderr`.
- Passing NULL for a session or handle pointer is always safe (returns
  `false` / `NULL` / empty string as appropriate).

---

## 8. Thread safety

**Not thread-safe.** A single session must not be accessed from multiple
threads concurrently. Create separate sessions per thread if concurrent access
is needed.

---

## 9. Known limitations

1. **Decorated name overloads**: when a class has multiple overloaded methods
   with the same name, only the first matching decorated name is returned.
2. **Template members**: template instantiation members are present but type
   names for template arguments may be verbose.
3. **Anonymous structs/unions**: unnamed members (e.g. anonymous union fields)
   will have empty names.
4. **Module symbol streams**: DBI per-module symbol streams are not read.
   Decorated names come from the PSI (public symbol table) only. Private
   symbols not in the PSI will have an empty `decorated_name`.
