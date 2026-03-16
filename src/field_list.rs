/// Extracts class members and member functions from a TPI field list.
///
/// Both operations start from the TypeIndex of the LF_CLASS/LF_STRUCTURE
/// record, then:
///   1. Read the record to get: total size, field_list TypeIndex, base class.
///   2. Call `type_stream.iter_fields(field_list_ti)` to walk the LF_FIELDLIST.
///   3. Match on Field variants to collect MemberInfo / FunctionInfo.

use ms_pdb::tpi::TypeStream;
use ms_pdb::codeview::types::kind::Leaf;
use ms_pdb::codeview::types::fields::Field;
use ms_pdb::codeview::types::records::{Struct as CvStruct, MemberFunc as CvMemberFunc};
use ms_pdb::codeview::parser::Parser;

use crate::type_name::{resolve_type_name, bstr_to_string};
use crate::symbol_stream::{SymbolIndex, lookup_decorated_names};
use crate::types::{ClassLayout, MemberInfo, FunctionInfo, ParamInfo};

/// Extract the full class layout (members + base class) for a UDT identified
/// by its TypeIndex.
///
/// Returns `None` if the record cannot be read or has no field list.
pub fn extract_class_layout(
    type_stream: &TypeStream<Vec<u8>>,
    class_name: &str,
    raw_ti: u32,
) -> Option<ClassLayout> {
    let ti = raw_ti.into();
    let record = type_stream.record(ti).ok()?;

    // Only process class / struct / interface records.
    if !matches!(
        record.kind,
        Leaf::LF_CLASS | Leaf::LF_STRUCTURE | Leaf::LF_INTERFACE
    ) {
        return None;
    }

    let mut parser = Parser::new(record.data);
    let s = CvStruct::from_parser(&mut parser).ok()?;

    let total_size = u32::try_from(s.length).unwrap_or(0);
    let field_list_ti: u32 = s.fixed.field_list.get().into();

    let mut layout = ClassLayout {
        class_name: class_name.to_owned(),
        base_class_name: String::new(),
        total_size,
        members: Vec::new(),
    };

    // Iterate the field list.
    // iter_fields follows LF_INDEX chains (chained field lists) automatically.
    for field in type_stream.iter_fields(field_list_ti.into()) {
        match field {
            // -------------------------------------------------------------- //
            // Data member
            // -------------------------------------------------------------- //
            Field::Member(m) => {
                let name = bstr_to_string(m.name);
                if name.is_empty() {
                    continue;
                }

                let member_ti: u32 = m.ty.into();
                let type_name = resolve_type_name(type_stream, member_ti, 0);
                let offset = i32::try_from(m.offset).unwrap_or(0);
                let size = size_of_type(type_stream, member_ti);

                layout.members.push(MemberInfo {
                    name,
                    type_name,
                    offset,
                    size,
                });
            }

            // -------------------------------------------------------------- //
            // Direct (non-virtual) base class — capture the first one only
            // (UE classes have single inheritance).
            // -------------------------------------------------------------- //
            Field::BaseClass(b) => {
                if layout.base_class_name.is_empty() {
                    let base_ti: u32 = b.ty.into();
                    let base_name = resolve_type_name(type_stream, base_ti, 0);
                    if !base_name.is_empty() && base_name != "unknown" {
                        layout.base_class_name = base_name;
                    }
                }
            }

            // -------------------------------------------------------------- //
            // Virtual base class — treat as base class if no direct base found.
            // -------------------------------------------------------------- //
            Field::DirectVirtualBaseClass(vb) => {
                if layout.base_class_name.is_empty() {
                    let base_ti: u32 = vb.fixed.btype.get().into();
                    let base_name = resolve_type_name(type_stream, base_ti, 0);
                    if !base_name.is_empty() && base_name != "unknown" {
                        layout.base_class_name = base_name;
                    }
                }
            }

            _ => {} // Ignore methods, nested types, friends, etc.
        }
    }

    Some(layout)
}

/// Extract all member functions for a UDT identified by its TypeIndex.
///
/// Returns an empty `Vec` if the type has no method entries.
pub fn extract_class_functions(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    class_name: &str,
    raw_ti: u32,
) -> Vec<FunctionInfo> {
    let ti = raw_ti.into();
    let record = match type_stream.record(ti) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    if !matches!(
        record.kind,
        Leaf::LF_CLASS | Leaf::LF_STRUCTURE | Leaf::LF_INTERFACE
    ) {
        return Vec::new();
    }

    let mut parser = Parser::new(record.data);
    let s = match CvStruct::from_parser(&mut parser) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let field_list_ti: u32 = s.fixed.field_list.get().into();
    let mut functions: Vec<FunctionInfo> = Vec::new();

    for field in type_stream.iter_fields(field_list_ti.into()) {
        match field {
            // -------------------------------------------------------------- //
            // Single method (no overloads)
            // -------------------------------------------------------------- //
            Field::OneMethod(m) => {
                let name = bstr_to_string(m.name);
                if should_skip_method(&name, class_name) {
                    continue;
                }

                let method_ti: u32 = m.ty.into();
                let attr = m.attr;

                if let Some(info) = resolve_method(
                    type_stream,
                    sym_index,
                    class_name,
                    &name,
                    method_ti,
                    attr,
                ) {
                    functions.push(info);
                }
            }

            // -------------------------------------------------------------- //
            // Overloaded method group (LF_METHOD → LF_METHODLIST)
            // -------------------------------------------------------------- //
            Field::Method(m) => {
                let name = bstr_to_string(m.name);
                if should_skip_method(&name, class_name) {
                    continue;
                }

                let method_list_ti: u32 = m.methods.into();
                let infos = resolve_method_list(
                    type_stream,
                    sym_index,
                    class_name,
                    &name,
                    method_list_ti,
                );
                functions.extend(infos);
            }

            _ => {} // Skip members, nested types, etc.
        }
    }

    functions
}

// -------------------------------------------------------------------------- //
// Internal helpers
// -------------------------------------------------------------------------- //

/// True if this method name should be excluded from the output.
/// Matches the filter logic in the LLVM and DIA backends.
fn should_skip_method(name: &str, class_name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    // Destructor
    if name.starts_with('~') {
        return true;
    }
    // Constructor (same name as class)
    if name == class_name {
        return true;
    }
    // Operator overloads
    if name.starts_with("operator") {
        return true;
    }
    false
}

/// Decode method attribute flags from CodeView CV_fldattr_t.
///
/// Layout (u16):
///   bits 1:0  = access  (1=private, 2=protected, 3=public)
///   bits 4:2  = mprop   (method property)
///   bit  5    = pseudo
///   bit  8    = compgenx
///   bit  9    = sealed
///
/// mprop values:
///   0 = vanilla, 1 = virtual, 2 = static, 3 = friend,
///   4 = intro virtual, 5 = pure virtual, 6 = pure intro virtual
fn attr_is_virtual(attr: u16) -> bool {
    matches!((attr >> 2) & 7, 1 | 4 | 5 | 6)
}

fn attr_is_static(attr: u16) -> bool {
    (attr >> 2) & 7 == 2
}

/// Resolve a single method given its LF_MFUNCTION TypeIndex.
fn resolve_method(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    class_name: &str,
    method_name: &str,
    method_type_ti: u32,
    attr: u16,
) -> Option<FunctionInfo> {
    let type_index = method_type_ti.into();
    let record = type_stream.record(type_index).ok()?;

    if record.kind != Leaf::LF_MFUNCTION {
        return None;
    }

    let mut parser = Parser::new(record.data);
    let mf: &CvMemberFunc = parser.get().ok()?;

    let return_ti: u32 = mf.return_value.get().into();
    let arg_list_ti: u32 = mf.arg_list.get().into();

    let return_type = resolve_type_name(type_stream, return_ti, 0);
    let params = extract_params(type_stream, arg_list_ti);

    // Look up decorated name from the public symbol index.
    let decorated_names = lookup_decorated_names(sym_index, class_name, method_name);
    let decorated_name = decorated_names.first().cloned().unwrap_or_default();

    Some(FunctionInfo {
        name: method_name.to_owned(),
        decorated_name,
        return_type,
        params,
        is_static: attr_is_static(attr),
        is_virtual: attr_is_virtual(attr),
        // is_const requires inspecting the `this` pointer type in LF_MFUNCTION.
        // The `this` field TypeIndex points to a LF_POINTER whose pointee
        // has LF_MODIFIER with the const bit set if the method is const.
        // This is implemented below via inspect_this_const.
        is_const: inspect_this_const(type_stream, mf),
    })
}

/// Resolve all methods from an LF_METHODLIST record.
fn resolve_method_list(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    class_name: &str,
    method_name: &str,
    method_list_ti: u32,
) -> Vec<FunctionInfo> {
    let type_index = method_list_ti.into();
    let record = match type_stream.record(type_index) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    if record.kind != Leaf::LF_METHODLIST {
        return Vec::new();
    }

    // LF_METHODLIST is a packed array of MethodListItem:
    //   attr:        u16
    //   _padding:    u16  (reserved)
    //   ty:          u32  (TypeIndex of LF_MFUNCTION)
    //   vtab_offset: u32  ONLY present when mprop is intro-virtual (4) or
    //                      pure-intro-virtual (6)
    //
    // Base entry = 8 bytes (attr + pad + ty). vtab_offset adds 4 more.
    let mut bytes = record.data;
    let mut results = Vec::new();

    while bytes.len() >= 8 {
        let attr = u16::from_le_bytes([bytes[0], bytes[1]]);
        // bytes[2..4] is padding
        let method_ti = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        let mprop = (attr >> 2) & 7;
        // Intro virtual (4) and pure intro virtual (6) have a vtab_offset field.
        let has_vtab_offset = mprop == 4 || mprop == 6;
        let entry_size = if has_vtab_offset { 12 } else { 8 };

        if let Some(info) = resolve_method(
            type_stream,
            sym_index,
            class_name,
            method_name,
            method_ti,
            attr,
        ) {
            results.push(info);
        }

        if bytes.len() < entry_size {
            break;
        }
        bytes = &bytes[entry_size..];
    }

    results
}

/// Extract the parameter list from an LF_ARGLIST record.
fn extract_params(type_stream: &TypeStream<Vec<u8>>, arg_list_ti: u32) -> Vec<ParamInfo> {
    if arg_list_ti == 0 {
        return Vec::new();
    }

    let type_index = arg_list_ti.into();
    let record = match type_stream.record(type_index) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    if record.kind != Leaf::LF_ARGLIST {
        return Vec::new();
    }

    // LF_ARGLIST layout: count (u32), then count × TypeIndex (u32 each)
    if record.data.len() < 4 {
        return Vec::new();
    }

    let count = u32::from_le_bytes([
        record.data[0],
        record.data[1],
        record.data[2],
        record.data[3],
    ]) as usize;

    let args_data = &record.data[4..];
    let mut params = Vec::with_capacity(count);

    for i in 0..count {
        let offset = i * 4;
        if offset + 4 > args_data.len() {
            break;
        }
        let arg_ti = u32::from_le_bytes([
            args_data[offset],
            args_data[offset + 1],
            args_data[offset + 2],
            args_data[offset + 3],
        ]);

        params.push(ParamInfo {
            name: format!("param{}", i),
            type_name: resolve_type_name(type_stream, arg_ti, 0),
        });
    }

    params
}

/// Determine whether a method is `const` by inspecting its `this` pointer type.
///
/// A const method has a `this` parameter of type `const T* const` in MSVC
/// encoding, which in CodeView is:
///   LF_MFUNCTION.this → TypeIndex of LF_POINTER
///   → LF_POINTER.pointee → TypeIndex of LF_MODIFIER with const bit set
///   → LF_MODIFIER.underlying → the class type
fn inspect_this_const(type_stream: &TypeStream<Vec<u8>>, mf: &CvMemberFunc) -> bool {
    let this_ti: u32 = mf.this.get().into();
    let begin_raw: u32 = type_stream.type_index_begin().into();

    if this_ti < begin_raw || this_ti == 0 {
        return false;
    }

    let this_record = match type_stream.record(this_ti.into()) {
        Ok(r) => r,
        Err(_) => return false,
    };

    if this_record.kind != Leaf::LF_POINTER {
        return false;
    }

    // The pointer's pointee type.
    if this_record.data.len() < 4 {
        return false;
    }
    let pointee_ti = u32::from_le_bytes([
        this_record.data[0],
        this_record.data[1],
        this_record.data[2],
        this_record.data[3],
    ]);

    let begin_raw: u32 = type_stream.type_index_begin().into();
    if pointee_ti < begin_raw {
        return false;
    }

    let modifier_record = match type_stream.record(pointee_ti.into()) {
        Ok(r) => r,
        Err(_) => return false,
    };

    if modifier_record.kind != Leaf::LF_MODIFIER {
        return false;
    }

    // LF_MODIFIER layout: underlying_type (u32), attributes (u16)
    // Attribute bits: bit 0 = const, bit 1 = volatile, bit 2 = unaligned
    if modifier_record.data.len() < 6 {
        return false;
    }
    let attr_bytes = [modifier_record.data[4], modifier_record.data[5]];
    let attr = u16::from_le_bytes(attr_bytes);
    attr & 1 != 0 // bit 0 = const
}

/// Best-effort size estimate for a type.
///
/// Used to fill `MemberInfo.size`. Returns 0 for complex types where the size
/// cannot be determined cheaply.
fn size_of_type(type_stream: &TypeStream<Vec<u8>>, ti: u32) -> u32 {
    let begin_raw: u32 = type_stream.type_index_begin().into();

    // Primitive types have predictable sizes.
    if ti < begin_raw {
        return primitive_size(ti);
    }

    let type_index = ti.into();
    let record = match type_stream.record(type_index) {
        Ok(r) => r,
        Err(_) => return 0,
    };

    match record.kind {
        Leaf::LF_CLASS | Leaf::LF_STRUCTURE | Leaf::LF_INTERFACE => {
            // The total size is the variable-length `length` field in the record.
            // Re-parse the Struct record to get it.
            let mut parser = Parser::new(record.data);
            if let Ok(s) = CvStruct::from_parser(&mut parser) {
                u32::try_from(s.length).unwrap_or(0)
            } else {
                0
            }
        }
        Leaf::LF_POINTER => 8, // x64: all pointers are 8 bytes
        Leaf::LF_ENUM => 4,    // enums are int32 by default in MSVC
        _ => 0,
    }
}

/// Size in bytes of a primitive CodeView type (TypeIndex < type_index_begin).
fn primitive_size(ti: u32) -> u32 {
    let mode = (ti >> 8) & 0xF;
    let base = ti & 0xFF;

    if mode > 0 {
        // Pointer mode — 8 bytes on x64, 4 on x86. We target x64.
        return 8;
    }

    match base {
        0x03 => 0,  // void
        0x30 => 1,  // bool
        0x10 | 0x20 | 0x60 | 0x61 | 0x68 | 0x69 => 1, // char variants
        0x11 | 0x21 => 2, // short, unsigned short
        0x12 | 0x22 | 0x74 | 0x75 => 4, // int, unsigned int
        0x13 | 0x23 | 0x76 | 0x77 => 8, // int64_t, uint64_t
        0x40 => 4,  // float
        0x41 => 8,  // double
        0x71 => 2,  // wchar_t
        _ => 0,
    }
}
