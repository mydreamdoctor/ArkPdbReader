/// Resolves a raw CodeView TypeIndex to a human-readable C++ type name string.
///
/// This mirrors the `ResolveTypeName` logic in `pdb_reader_llvm.cpp` and
/// `pdb_reader.cpp` (DIA version), but works entirely from raw TPI records.
///
/// Supported forms:
///   - Primitive types (TypeIndex < type_index_begin)
///   - LF_POINTER  → "T*" or "T&" or "T&&"
///   - LF_MODIFIER → strips const/volatile, recurses on underlying type
///   - LF_ARRAY    → "T[]"
///   - LF_CLASS / LF_STRUCTURE / LF_UNION → type name from record
///   - LF_ENUM     → enum name from record
///   - LF_MFUNCTION / LF_PROCEDURE → "void(*)()"
///   - Everything else → "unknown"

use ms_pdb::tpi::TypeStream;
use ms_pdb::codeview::types::kind::Leaf;
use ms_pdb::codeview::types::records::{Struct, MemberFunc};
use ms_pdb::codeview::types::iter::TypeRecord;
use ms_pdb::codeview::parser::Parser;
use bstr::BStr;

/// Maximum recursion depth when resolving nested type references.
/// Guards against malformed or circular PDB records.
const MAX_DEPTH: u32 = 12;

/// Resolve a CodeView TypeIndex to a C++ type name.
///
/// `ti` is a raw u32. Values below `type_stream.type_index_begin()` are
/// CodeView primitives and are resolved directly without a TPI lookup.
pub fn resolve_type_name(type_stream: &TypeStream<Vec<u8>>, ti: u32, depth: u32) -> String {
    if depth >= MAX_DEPTH {
        return "...".to_string();
    }

    // Primitive TypeIndex values are encoded directly in the index value.
    // They are always below the TPI stream's type_index_begin.
    let begin = type_stream.type_index_begin();
    // TypeIndex might be a newtype wrapping u32 — extract the raw value.
    // If TypeIndex IS u32 this cast is a no-op.
    let begin_raw: u32 = begin.into();

    if ti < begin_raw {
        return resolve_primitive(ti);
    }

    // Construct the TypeIndex value the way ms-pdb expects it.
    // If TypeIndex is a plain u32 alias this is a no-op.
    // If it is a newtype we rely on From<u32> being implemented.
    let type_index = ti.into();

    match type_stream.record(type_index) {
        Ok(record) => resolve_record(type_stream, &record, depth + 1),
        Err(_) => "unknown".to_string(),
    }
}

fn resolve_record(
    type_stream: &TypeStream<Vec<u8>>,
    record: &TypeRecord<'_>,
    depth: u32,
) -> String {
    match record.kind {
        // ------------------------------------------------------------------ //
        // UDT types: return their stored name directly.
        // ------------------------------------------------------------------ //
        Leaf::LF_CLASS | Leaf::LF_STRUCTURE | Leaf::LF_INTERFACE => {
            parse_struct_name(record.data).unwrap_or_else(|| "unknown".to_string())
        }
        Leaf::LF_UNION => {
            parse_struct_name(record.data).unwrap_or_else(|| "unknown".to_string())
        }
        Leaf::LF_ENUM => {
            parse_enum_name(record.data).unwrap_or_else(|| "unknown".to_string())
        }

        // ------------------------------------------------------------------ //
        // Pointer: append * or & to the pointee type name.
        // ------------------------------------------------------------------ //
        Leaf::LF_POINTER => {
            parse_pointer_name(type_stream, record.data, depth)
        }

        // ------------------------------------------------------------------ //
        // Modifier: const / volatile wrapper - resolve the underlying type.
        // We strip const/volatile from the name since we only care about the
        // base type for generator purposes.
        // ------------------------------------------------------------------ //
        Leaf::LF_MODIFIER => {
            parse_modifier_name(type_stream, record.data, depth)
        }

        // ------------------------------------------------------------------ //
        // Array: element type + "[]"
        // ------------------------------------------------------------------ //
        Leaf::LF_ARRAY => {
            parse_array_name(type_stream, record.data, depth)
        }

        // ------------------------------------------------------------------ //
        // Function types: emit a generic signature placeholder.
        // The generator does not need full function-pointer type expansion.
        // ------------------------------------------------------------------ //
        Leaf::LF_MFUNCTION | Leaf::LF_PROCEDURE => "void(*)()".to_string(),

        // ------------------------------------------------------------------ //
        // Typedef: look through to the underlying type.
        // ------------------------------------------------------------------ //
        Leaf::LF_ALIAS => {
            parse_alias_name(type_stream, record.data, depth)
        }

        // ------------------------------------------------------------------ //
        // Bitfield: resolve to the underlying integer type.
        // ------------------------------------------------------------------ //
        Leaf::LF_BITFIELD => {
            parse_bitfield_name(type_stream, record.data, depth)
        }

        _ => "unknown".to_string(),
    }
}

// -------------------------------------------------------------------------- //
// Per-leaf parsers
// -------------------------------------------------------------------------- //

fn parse_struct_name(data: &[u8]) -> Option<String> {
    // Struct / Union share the same header layout (StructFixed / UnionFixed):
    //   num_elements: U16
    //   property:     U16  (UdtPropertiesLe)
    //   field_list:   U32  (TypeIndexLe)
    //   derivation_list: U32 (TypeIndexLe)   -- Struct only
    //   vtable_shape:    U32 (TypeIndexLe)   -- Struct only
    //   length:  variable-length Number
    //   name:    null-terminated BStr
    //
    // We only need the name. Skip past the fixed header and the length number
    // using the codeview Parser.
    let mut parser = Parser::new(data);
    // Skip num_elements (2) + property (2) + field_list (4)
    // + derivation_list (4) + vtable_shape (4) = 16 bytes for LF_CLASS/STRUCT
    // For LF_UNION: skip count (2) + property (2) + field_list (4) = 8 bytes
    // We use a try-catch approach: parse the Struct record properly.
    use ms_pdb::codeview::types::records::Struct as CvStruct;
    if let Ok(s) = CvStruct::from_parser(&mut parser) {
        let name = bstr_to_string(s.name);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn parse_enum_name(data: &[u8]) -> Option<String> {
    // EnumFixed: count (U16), property (U16), underlying_type (U32), fields (U32)
    // Then: name (BStr null-terminated)
    use ms_pdb::codeview::types::records::Enum as CvEnum;
    let mut parser = Parser::new(data);
    if let Ok(e) = CvEnum::from_parser(&mut parser) {
        let name = bstr_to_string(e.name);
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn parse_pointer_name(
    type_stream: &TypeStream<Vec<u8>>,
    data: &[u8],
    depth: u32,
) -> String {
    // PointerFixed: ty (U32 TypeIndexLe), attr (U32)
    // attr bits encode: pointer kind, size, const/volatile, reference-ness
    use ms_pdb::codeview::types::records::Pointer as CvPointer;
    let mut parser = Parser::new(data);
    if let Ok(p) = CvPointer::from_parser(&mut parser) {
        let pointee_ti: u32 = p.fixed.ty.get().into();
        let attr: u32 = p.fixed.attr.get();

        // Bit 5 = is_reference (lvalue ref), bit 6 = is_rvalue_reference
        let is_ref = (attr >> 5) & 1 != 0;
        let is_rval_ref = (attr >> 6) & 1 != 0;

        let base_name = resolve_type_name(type_stream, pointee_ti, depth);
        if is_rval_ref {
            base_name + "&&"
        } else if is_ref {
            base_name + "&"
        } else {
            base_name + "*"
        }
    } else {
        "void*".to_string()
    }
}

fn parse_modifier_name(
    type_stream: &TypeStream<Vec<u8>>,
    data: &[u8],
    depth: u32,
) -> String {
    // TypeModifier: underlying_type (U32 TypeIndexLe), attributes (U16)
    use ms_pdb::codeview::types::records::TypeModifier as CvModifier;
    let mut parser = Parser::new(data);
    if let Ok(m) = CvModifier::from_parser(&mut parser) {
        let inner_ti: u32 = m.underlying_type.get().into();
        resolve_type_name(type_stream, inner_ti, depth)
    } else {
        "unknown".to_string()
    }
}

fn parse_array_name(
    type_stream: &TypeStream<Vec<u8>>,
    data: &[u8],
    depth: u32,
) -> String {
    // ArrayFixed: element_type (U32 TypeIndexLe), index_type (U32 TypeIndexLe)
    // Then: len (Number), name (BStr)
    use ms_pdb::codeview::types::records::Array as CvArray;
    let mut parser = Parser::new(data);
    if let Ok(a) = CvArray::from_parser(&mut parser) {
        let elem_ti: u32 = a.fixed.element_type.get().into();
        resolve_type_name(type_stream, elem_ti, depth) + "[]"
    } else {
        "unknown[]".to_string()
    }
}

fn parse_alias_name(
    type_stream: &TypeStream<Vec<u8>>,
    data: &[u8],
    depth: u32,
) -> String {
    // LF_ALIAS (typedef): aliasee_type (U32 TypeIndexLe), then name
    // The exact layout depends on the codeview version; parse carefully.
    // A typedef may have: underlying TypeIndex at offset 0, then a name.
    // We try to extract the underlying type and recurse.
    if data.len() >= 4 {
        let inner_ti = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        return resolve_type_name(type_stream, inner_ti, depth);
    }
    "unknown".to_string()
}

fn parse_bitfield_name(
    type_stream: &TypeStream<Vec<u8>>,
    data: &[u8],
    depth: u32,
) -> String {
    // Bitfield: underlying_type (U32 TypeIndexLe), length (u8), position (u8)
    use ms_pdb::codeview::types::records::Bitfield as CvBitfield;
    let mut parser = Parser::new(data);
    if let Ok(b) = CvBitfield::from_parser(&mut parser) {
        let inner_ti: u32 = b.underlying_type.get().into();
        resolve_type_name(type_stream, inner_ti, depth)
    } else if data.len() >= 4 {
        let inner_ti = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        resolve_type_name(type_stream, inner_ti, depth)
    } else {
        "unknown".to_string()
    }
}

// -------------------------------------------------------------------------- //
// Primitive type resolution
//
// CodeView primitive TypeIndex layout (when TypeIndex < type_index_begin):
//   bits 7:0  = base type code (CVBASETYPE)
//   bits 11:8 = pointer mode (0 = direct, 6 = near 64-bit pointer, etc.)
// -------------------------------------------------------------------------- //

pub fn resolve_primitive(ti: u32) -> String {
    let mode = (ti >> 8) & 0xF;
    let base = ti & 0xFF;

    if mode > 0 {
        // Pointer to the base type — resolve base and append *.
        // Mode 6 = near 64-bit pointer (x64 native); others are 16/32-bit.
        return resolve_primitive_base(base) + "*";
    }

    resolve_primitive_base(base)
}

fn resolve_primitive_base(base: u32) -> String {
    match base {
        0x00 => "void".to_string(),    // T_NOTYPE / none
        0x03 => "void".to_string(),    // T_VOID
        0x08 => "HRESULT".to_string(), // T_HRESULT
        0x10 => "char".to_string(),    // T_CHAR (signed char)
        0x20 => "unsigned char".to_string(), // T_UCHAR
        0x30 => "bool".to_string(),    // T_BOOL08
        0x40 => "float".to_string(),   // T_REAL32
        0x41 => "double".to_string(),  // T_REAL64
        0x42 => "long double".to_string(), // T_REAL80
        0x60 => "int8_t".to_string(),  // T_INT1
        0x61 => "uint8_t".to_string(), // T_UINT1
        0x68 => "int8_t".to_string(),  // T_RCHAR (signed 1-byte int)
        0x69 => "uint8_t".to_string(), // T_RCHAR variant
        0x70 => "wchar_t".to_string(), // T_WCHAR
        0x71 => "wchar_t".to_string(), // T_WCHAR (some encodings)
        0x11 => "short".to_string(),   // T_SHORT
        0x21 => "unsigned short".to_string(), // T_USHORT
        0x12 | 0x74 => "int".to_string(),          // T_LONG / T_INT4
        0x22 | 0x75 => "unsigned int".to_string(),  // T_ULONG / T_UINT4
        0x13 | 0x76 => "int64_t".to_string(),       // T_QUAD / T_INT8
        0x23 | 0x77 => "uint64_t".to_string(),      // T_UQUAD / T_UINT8
        0x14 => "int128_t".to_string(), // T_OCT
        0x24 => "uint128_t".to_string(), // T_UOCT
        0x73 => "char16_t".to_string(), // T_CHAR16
        0x7a => "char32_t".to_string(), // T_CHAR32
        0x7b => "char8_t".to_string(),  // T_CHAR8
        _ => format!("T_{:02x}", base), // unknown primitive
    }
}

// -------------------------------------------------------------------------- //
// Helpers
// -------------------------------------------------------------------------- //

/// Convert a `&BStr` (byte string from the PDB) to a Rust `String`.
/// PDB names are typically ASCII or UTF-8; invalid bytes are replaced with '?'.
pub fn bstr_to_string(s: &BStr) -> String {
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}
