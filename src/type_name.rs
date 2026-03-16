/// Resolves a CodeView TypeIndex to a human-readable C++ type name string.

use ms_pdb::tpi::TypeStream;
use ms_pdb::types::{TypeData, TypeIndex};

const MAX_DEPTH: u32 = 8;

/// Resolve TypeIndex `ti` to a C++ type name string.
pub fn resolve_type_name(ts: &TypeStream<Vec<u8>>, ti: TypeIndex, depth: u32) -> String {
    if depth > MAX_DEPTH {
        return "<...>".to_string();
    }

    // Primitives live below type_index_begin (0x1000).
    if ti.0 < TypeIndex::MIN_BEGIN.0 {
        return resolve_primitive(ti.0);
    }

    let Ok(record) = ts.record(ti) else {
        return format!("<T#{:#x}>", ti.0);
    };
    let Ok(td) = record.parse() else {
        return format!("<T#{:#x}>", ti.0);
    };

    match td {
        TypeData::Struct(s) => s.name.to_string(),
        TypeData::Union(u) => u.name.to_string(),
        TypeData::Enum(e) => e.name.to_string(),
        TypeData::Alias(a) => a.name.to_string(),

        TypeData::Pointer(p) => {
            let inner_ti = p.fixed.ty.get();
            let flags = p.fixed.attr();
            let inner = resolve_type_name(ts, inner_ti, depth + 1);
            if flags.islref() {
                format!("{}&", inner)
            } else if flags.isrref() {
                format!("{}&&", inner)
            } else {
                format!("{}*", inner)
            }
        }

        TypeData::Modifier(m) => {
            let inner = resolve_type_name(ts, m.underlying_type.get(), depth + 1);
            if m.is_const() {
                format!("const {}", inner)
            } else {
                inner
            }
        }

        TypeData::Array(a) => {
            let elem = resolve_type_name(ts, a.fixed.element_type.get(), depth + 1);
            format!("{}[]", elem)
        }

        TypeData::MemberFunc(mf) => {
            let ret = resolve_type_name(ts, mf.return_value.get(), depth + 1);
            format!("{} (__thiscall*)()", ret)
        }

        TypeData::Proc(p) => {
            let ret = resolve_type_name(ts, p.return_value.get(), depth + 1);
            format!("{} (*)()", ret)
        }

        TypeData::Bitfield(b) => {
            let inner = resolve_type_name(ts, b.underlying_type.get(), depth + 1);
            format!("{}:{}", inner, b.length)
        }

        _ => format!("<T#{:#x}>", ti.0),
    }
}

/// Resolve a primitive TypeIndex (< 0x1000) to a C++ name.
///
/// CodeView primitive encoding:
///   bits 7:0  = base type code
///   bits 11:8 = pointer mode (0 = direct value, non-zero = pointer)
fn resolve_primitive(ti: u32) -> String {
    let mode = (ti >> 8) & 0xF;
    let base = ti & 0xFF;

    let base_name = match base {
        0x00 | 0x01 | 0x02 => "void",
        0x03 => "void",
        0x08 => "HRESULT",
        0x10 => "signed char",
        0x11 => "short",
        0x12 => "long",
        0x13 => "long long",
        0x14 => "__int128",
        0x20 => "unsigned char",
        0x21 => "unsigned short",
        0x22 => "unsigned long",
        0x23 => "unsigned long long",
        0x24 => "unsigned __int128",
        0x30 => "bool",
        0x40 => "float",
        0x41 => "double",
        0x42 => "long double",
        0x60 => "__int8",
        0x61 => "unsigned __int8",
        0x68 => "char",
        0x69 => "unsigned char",
        0x70 => "wchar_t",
        0x71 => "char16_t",
        0x72 => "__int16",
        0x73 => "unsigned __int16",
        0x74 => "int",
        0x75 => "unsigned int",
        0x76 => "__int64",
        0x77 => "unsigned __int64",
        0x78 => "__int128",
        0x79 => "unsigned __int128",
        0x7a => "char16_t",
        0x7b => "char32_t",
        0x7c => "char8_t",
        _ => return format!("<prim#0x{:04x}>", ti),
    };

    if mode != 0 {
        format!("{}*", base_name)
    } else {
        base_name.to_string()
    }
}

/// Convert a `&bstr::BStr` to an owned `String`.
pub fn bstr_to_string(s: &bstr::BStr) -> String {
    String::from_utf8_lossy(s.as_ref()).into_owned()
}
