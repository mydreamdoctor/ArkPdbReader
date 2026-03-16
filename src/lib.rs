/// ArkPdbReader — public C FFI surface.
///
/// All functions in this module are `extern "C"` and `#[no_mangle]` so they
/// can be called from C and C++ without name mangling.
///
/// # Ownership model
///
/// `ArkPdbSession` is an opaque pointer to a heap-allocated `Session`.
/// Layout and function results are returned as opaque handle pointers
/// (`ArkLayoutHandle`, `ArkFunctionListHandle`) which are separately freed.
/// The session itself does not own result handles; they can outlive each other.
///
/// Caller must:
/// - Create: `ark_pdb_open`  → `ArkPdbSession*`
/// - Destroy: `ark_pdb_close`
/// - Create result: `ark_pdb_find_class_layout` → `ArkLayoutHandle*`
/// - Destroy result: `ark_pdb_layout_free`
/// - Same pattern for `ArkFunctionListHandle`.
///
/// # Safety
///
/// All FFI functions wrap their bodies in `std::panic::catch_unwind`.
/// A panic returns null / false and logs to stderr.
///
/// All `const char*` parameters must be valid non-null null-terminated
/// UTF-8 C strings for the duration of the call.

mod field_list;
mod session;
mod symbol_stream;
mod type_index;
mod type_name;
mod types;

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic;

use session::Session;
pub use types::*;

// ============================================================================
// Session lifecycle
// ============================================================================

/// Open a PDB file and prepare it for querying.
///
/// Reads the TPI stream and Global Symbol Stream into memory on open.
/// All subsequent queries use only in-memory data.
///
/// Returns null on failure; use stderr output for diagnostics (the session
/// does not exist yet, so there is no error handle).
#[no_mangle]
pub extern "C" fn ark_pdb_open(path: *const c_char) -> *mut Session {
    let path_str = match to_rust_str(path) {
        Some(s) => s,
        None => {
            eprintln!("[ArkPdbReader] ark_pdb_open: invalid path");
            return std::ptr::null_mut();
        }
    };

    match panic::catch_unwind(|| Session::open(path_str)) {
        Ok(Ok(session)) => Box::into_raw(Box::new(session)),
        Ok(Err(e)) => {
            eprintln!("[ArkPdbReader] ark_pdb_open failed: {e:#}");
            std::ptr::null_mut()
        }
        Err(_) => {
            eprintln!("[ArkPdbReader] ark_pdb_open panicked");
            std::ptr::null_mut()
        }
    }
}

/// Free a session created by `ark_pdb_open`.  Null is safe and does nothing.
#[no_mangle]
pub extern "C" fn ark_pdb_close(session: *mut Session) {
    if !session.is_null() {
        unsafe { drop(Box::from_raw(session)) };
    }
}

/// Return the last error string stored on the session.
///
/// The pointer is valid until the next mutable call on this session or
/// until `ark_pdb_close`.  The caller must NOT free it.
/// Returns a pointer to an empty string if there is no error or session is null.
#[no_mangle]
pub extern "C" fn ark_pdb_last_error(session: *const Session) -> *const c_char {
    static EMPTY: &[u8] = b"\0";
    if session.is_null() {
        return EMPTY.as_ptr() as *const c_char;
    }
    let s = unsafe { &*session };
    // last_error is a plain String; return its bytes + a null from a CString
    // stored on the session.  We store a CString in the session for this.
    s.last_error_cstr.as_ptr()
}

// ============================================================================
// Class name enumeration
// ============================================================================

/// Callback called once per class name in `ark_pdb_list_class_names`.
/// Return `true` to continue, `false` to stop early.
pub type ArkClassNameCallback = unsafe extern "C" fn(
    name: *const c_char,
    user_data: *mut std::ffi::c_void,
) -> bool;

/// Enumerate all Unreal Engine–style top-level class names from the PDB.
///
/// Names pass the same prefix filter as the ArkSdkGen LLVM backend:
/// `[A|U|F|E|T|I][A-Z]...` with no templates (`<`) or namespaces (`::`).
///
/// Names are delivered in sorted alphabetical order.
///
/// Returns `true` on success, `false` on error.
#[no_mangle]
pub extern "C" fn ark_pdb_list_class_names(
    session: *mut Session,
    callback: ArkClassNameCallback,
    user_data: *mut std::ffi::c_void,
) -> bool {
    ffi_guard(session, |s| {
        let index = s.name_index();
        let mut names: Vec<&str> = index
            .values()
            .filter(|e| type_index::is_ue_top_level_class(&e.canonical_name))
            .map(|e| e.canonical_name.as_str())
            .collect();
        names.sort_unstable();

        for name in names {
            if let Ok(c) = CString::new(name) {
                if !unsafe { callback(c.as_ptr(), user_data) } {
                    break;
                }
            }
        }
        true
    })
}

// ============================================================================
// Type existence check
// ============================================================================

/// Check whether a type name exists in the PDB (case-insensitive).
///
/// If found and `out_resolved_name` is non-null, writes the canonical
/// (exact-case) name into the caller-supplied buffer (null-terminated,
/// truncated if needed).
///
/// Returns `true` if found, `false` otherwise.
#[no_mangle]
pub extern "C" fn ark_pdb_type_exists(
    session: *mut Session,
    name: *const c_char,
    out_resolved_name: *mut c_char,
    buf_len: usize,
) -> bool {
    let name_str = match to_rust_str(name) {
        Some(s) => s,
        None => return false,
    };

    ffi_guard(session, |s| {
        match type_index::lookup_name(s.name_index(), name_str) {
            Some(entry) => {
                write_cstr(out_resolved_name, buf_len, &entry.canonical_name);
                true
            }
            None => false,
        }
    })
}

// ============================================================================
// Class layout  (opaque handle API)
// ============================================================================

/// Opaque handle holding the layout result for one class.
/// Created by `ark_pdb_find_class_layout`, freed by `ark_pdb_layout_free`.
pub struct ArkLayoutHandle {
    layout: ClassLayout,
}

/// Find the layout of a named class or struct.
///
/// Returns an opaque handle on success or null on failure.
/// The handle must be freed with `ark_pdb_layout_free`.
/// Lookup is case-insensitive.
#[no_mangle]
pub extern "C" fn ark_pdb_find_class_layout(
    session: *mut Session,
    class_name: *const c_char,
) -> *mut ArkLayoutHandle {
    let name_str = match to_rust_str(class_name) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let result = ffi_guard_opt(session, |s| {
        // Check session layout cache first.
        if let Some(layout) = s.layout_cache.get(name_str) {
            return Some(layout.clone());
        }

        let type_idx = type_index::lookup_name(s.name_index(), name_str)?.type_index;
        let layout = field_list::extract_class_layout(&s.type_stream, name_str, type_idx)?;
        s.layout_cache.insert(name_str.to_owned(), layout.clone());
        Some(layout)
    });

    match result {
        Some(layout) => Box::into_raw(Box::new(ArkLayoutHandle { layout })),
        None => std::ptr::null_mut(),
    }
}

/// Free a layout handle returned by `ark_pdb_find_class_layout`.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_free(handle: *mut ArkLayoutHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

/// Write the base class name into `buf` (null-terminated, truncated if needed).
/// Writes an empty string if the class has no base class.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_base_class(
    handle: *const ArkLayoutHandle,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    write_cstr(buf, buf_len, &h.layout.base_class_name);
}

/// Return the total size of the struct/class in bytes.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_total_size(handle: *const ArkLayoutHandle) -> u32 {
    unsafe { (*handle).layout.total_size }
}

/// Return the number of data members in the layout.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_member_count(handle: *const ArkLayoutHandle) -> i32 {
    unsafe { (*handle).layout.members.len() as i32 }
}

/// Write the field name of member at `index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_member_name(
    handle: *const ArkLayoutHandle,
    index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(m) = h.layout.members.get(index as usize) {
        write_cstr(buf, buf_len, &m.name);
    }
}

/// Write the C++ type name of member at `index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_member_type(
    handle: *const ArkLayoutHandle,
    index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(m) = h.layout.members.get(index as usize) {
        write_cstr(buf, buf_len, &m.type_name);
    }
}

/// Return the byte offset of member at `index` from the start of the struct.
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_member_offset(
    handle: *const ArkLayoutHandle,
    index: i32,
) -> i32 {
    let h = unsafe { &*handle };
    h.layout.members.get(index as usize).map_or(0, |m| m.offset)
}

/// Return the size in bytes of member at `index` (0 = unknown).
#[no_mangle]
pub extern "C" fn ark_pdb_layout_get_member_size(
    handle: *const ArkLayoutHandle,
    index: i32,
) -> u32 {
    let h = unsafe { &*handle };
    h.layout.members.get(index as usize).map_or(0, |m| m.size)
}

// ============================================================================
// Class functions  (opaque handle API)
// ============================================================================

/// Opaque handle holding the function list for one class.
/// Created by `ark_pdb_find_class_functions`, freed by `ark_pdb_funclist_free`.
pub struct ArkFunctionListHandle {
    functions: Vec<FunctionInfo>,
}

/// Find all member functions of a named class or struct.
///
/// Excludes constructors, destructors, and operator overloads.
/// Returns an opaque handle on success or null on failure (including when the
/// class has no methods).
/// The handle must be freed with `ark_pdb_funclist_free`.
/// Lookup is case-insensitive.
#[no_mangle]
pub extern "C" fn ark_pdb_find_class_functions(
    session: *mut Session,
    class_name: *const c_char,
) -> *mut ArkFunctionListHandle {
    let name_str = match to_rust_str(class_name) {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let result = ffi_guard_opt(session, |s| {
        if let Some(funcs) = s.function_cache.get(name_str) {
            return Some(funcs.clone());
        }

        let type_idx = type_index::lookup_name(s.name_index(), name_str)?.type_index;
        let sym_index = s.symbol_index();
        let funcs = field_list::extract_class_functions(
            &s.type_stream,
            sym_index,
            name_str,
            type_idx,
        );
        s.function_cache.insert(name_str.to_owned(), funcs.clone());
        Some(funcs)
    });

    match result {
        Some(functions) => {
            Box::into_raw(Box::new(ArkFunctionListHandle { functions }))
        }
        None => std::ptr::null_mut(),
    }
}

/// Free a function list handle returned by `ark_pdb_find_class_functions`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_free(handle: *mut ArkFunctionListHandle) {
    if !handle.is_null() {
        unsafe { drop(Box::from_raw(handle)) };
    }
}

/// Return the number of functions in the list.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_count(handle: *const ArkFunctionListHandle) -> i32 {
    unsafe { (*handle).functions.len() as i32 }
}

/// Write the short name (e.g. `GetPlayerName`) of function at `index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_name(
    handle: *const ArkFunctionListHandle,
    index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(f) = h.functions.get(index as usize) {
        write_cstr(buf, buf_len, &f.name);
    }
}

/// Write the decorated (mangled) name of function at `index` into `buf`.
/// Writes an empty string if the decorated name could not be resolved.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_decorated_name(
    handle: *const ArkFunctionListHandle,
    index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(f) = h.functions.get(index as usize) {
        write_cstr(buf, buf_len, &f.decorated_name);
    }
}

/// Write the C++ return type of function at `index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_return_type(
    handle: *const ArkFunctionListHandle,
    index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(f) = h.functions.get(index as usize) {
        write_cstr(buf, buf_len, &f.return_type);
    }
}

/// Return `true` if function at `index` is static.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_is_static(handle: *const ArkFunctionListHandle, index: i32) -> bool {
    let h = unsafe { &*handle };
    h.functions.get(index as usize).map_or(false, |f| f.is_static)
}

/// Return `true` if function at `index` is virtual or pure virtual.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_is_virtual(handle: *const ArkFunctionListHandle, index: i32) -> bool {
    let h = unsafe { &*handle };
    h.functions.get(index as usize).map_or(false, |f| f.is_virtual)
}

/// Return `true` if function at `index` is const-qualified.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_is_const(handle: *const ArkFunctionListHandle, index: i32) -> bool {
    let h = unsafe { &*handle };
    h.functions.get(index as usize).map_or(false, |f| f.is_const)
}

/// Return the number of parameters of function at `func_index`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_param_count(handle: *const ArkFunctionListHandle, func_index: i32) -> i32 {
    let h = unsafe { &*handle };
    h.functions.get(func_index as usize).map_or(0, |f| f.params.len() as i32)
}

/// Write the name of parameter `param_index` of function `func_index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_param_name(
    handle: *const ArkFunctionListHandle,
    func_index: i32,
    param_index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(f) = h.functions.get(func_index as usize) {
        if let Some(p) = f.params.get(param_index as usize) {
            write_cstr(buf, buf_len, &p.name);
        }
    }
}

/// Write the type name of parameter `param_index` of function `func_index` into `buf`.
#[no_mangle]
pub extern "C" fn ark_pdb_funclist_get_param_type(
    handle: *const ArkFunctionListHandle,
    func_index: i32,
    param_index: i32,
    buf: *mut c_char,
    buf_len: usize,
) {
    let h = unsafe { &*handle };
    if let Some(f) = h.functions.get(func_index as usize) {
        if let Some(p) = f.params.get(param_index as usize) {
            write_cstr(buf, buf_len, &p.type_name);
        }
    }
}

// ============================================================================
// Internal utilities
// ============================================================================

/// Run a closure over `&mut Session`, returning `false` if null or on panic.
///
/// `&mut Session` is not `UnwindSafe` (it contains `HashMap` etc.) so we
/// wrap it in `AssertUnwindSafe`.  The safety contract: we do not use the
/// session after a panic (the session pointer becomes inaccessible to the
/// caller on a `false` return).
fn ffi_guard<F>(session: *mut Session, f: F) -> bool
where
    F: FnOnce(&mut Session) -> bool,
{
    if session.is_null() {
        return false;
    }
    let s = unsafe { &mut *session };
    match panic::catch_unwind(panic::AssertUnwindSafe(|| f(s))) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("[ArkPdbReader] caught panic");
            false
        }
    }
}

/// Like `ffi_guard` but returns an `Option<T>` — used for handle-returning functions.
fn ffi_guard_opt<T, F>(session: *mut Session, f: F) -> Option<T>
where
    F: FnOnce(&mut Session) -> Option<T>,
{
    if session.is_null() {
        return None;
    }
    let s = unsafe { &mut *session };
    match panic::catch_unwind(panic::AssertUnwindSafe(|| f(s))) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("[ArkPdbReader] caught panic");
            None
        }
    }
}

/// Safely convert a `*const c_char` to `&str`.  Returns `None` if null or
/// the bytes are not valid UTF-8.
fn to_rust_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Write a Rust `&str` into a `*mut c_char` buffer, null-terminating and
/// truncating if necessary.  A null `buf` or `buf_len == 0` is safe (no-op).
fn write_cstr(buf: *mut c_char, buf_len: usize, value: &str) {
    if buf.is_null() || buf_len == 0 {
        return;
    }
    let bytes = value.as_bytes();
    let write_len = (buf_len - 1).min(bytes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, write_len);
        *buf.add(write_len) = 0;
    }
}
