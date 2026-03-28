/// Extracts class members and member functions from a TPI field list.
use ms_pdb::tpi::TypeStream;
use ms_pdb::types::{fields::Field, MethodList, TypeData, TypeIndex};

use crate::proc_params::{choose_best_owner_match, choose_best_public_match, ProcParamIndex};
use crate::symbol_stream::{lookup_decorated_names, SymbolIndex};
use crate::type_name::{bstr_to_string, resolve_type_name};
use crate::types::{ClassLayout, FunctionInfo, MemberInfo, ParamInfo};

/// Extract the full class layout (data members + base class) for a UDT.
pub fn extract_class_layout(
    type_stream: &TypeStream<Vec<u8>>,
    class_name: &str,
    type_index: TypeIndex,
) -> Option<ClassLayout> {
    let record = type_stream.record(type_index).ok()?;

    let td = record.parse().ok()?;
    let s = match &td {
        TypeData::Struct(s) => s,
        _ => return None,
    };

    let total_size = u32::try_from(s.length).unwrap_or(0);
    let field_list_ti = s.fixed.field_list.get();

    let mut layout = ClassLayout {
        class_name: class_name.to_owned(),
        base_class_name: String::new(),
        total_size,
        members: Vec::new(),
    };

    for field in type_stream.iter_fields(field_list_ti) {
        match field {
            Field::Member(m) => {
                let name = bstr_to_string(m.name);
                if name.is_empty() {
                    continue;
                }
                let type_name = resolve_type_name(type_stream, m.ty, 0);
                let offset = i32::try_from(m.offset).unwrap_or(0);
                let size = size_of_type(type_stream, m.ty);
                layout.members.push(MemberInfo {
                    name,
                    type_name,
                    offset,
                    size,
                });
            }

            Field::BaseClass(b) => {
                if layout.base_class_name.is_empty() {
                    let name = resolve_type_name(type_stream, b.ty, 0);
                    if !name.is_empty() && !name.starts_with('<') {
                        layout.base_class_name = name;
                    }
                }
            }

            Field::DirectVirtualBaseClass(vb) => {
                if layout.base_class_name.is_empty() {
                    let name = resolve_type_name(type_stream, vb.fixed.btype.get(), 0);
                    if !name.is_empty() && !name.starts_with('<') {
                        layout.base_class_name = name;
                    }
                }
            }

            _ => {}
        }
    }

    Some(layout)
}

/// Extract all member functions for a UDT.
pub fn extract_class_functions(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    proc_param_index: &ProcParamIndex,
    class_name: &str,
    type_index: TypeIndex,
) -> Vec<FunctionInfo> {
    let record = match type_stream.record(type_index) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let td = match record.parse() {
        Ok(td) => td,
        Err(_) => return Vec::new(),
    };

    let field_list_ti = match &td {
        TypeData::Struct(s) => s.fixed.field_list.get(),
        _ => return Vec::new(),
    };

    let mut functions: Vec<FunctionInfo> = Vec::new();

    for field in type_stream.iter_fields(field_list_ti) {
        match field {
            Field::OneMethod(m) => {
                let name = bstr_to_string(m.name);
                if should_skip_method(&name, class_name) {
                    continue;
                }
                if let Some(info) = resolve_method(
                    type_stream,
                    sym_index,
                    proc_param_index,
                    class_name,
                    &name,
                    m.ty,
                    m.attr,
                ) {
                    functions.push(info);
                }
            }

            Field::Method(m) => {
                let name = bstr_to_string(m.name);
                if should_skip_method(&name, class_name) {
                    continue;
                }
                let infos = resolve_method_list(
                    type_stream,
                    sym_index,
                    proc_param_index,
                    class_name,
                    &name,
                    m.methods,
                );
                functions.extend(infos);
            }

            _ => {}
        }
    }

    functions
}

// ── helpers ────────────────────────────────────────────────────────────────

fn should_skip_method(name: &str, class_name: &str) -> bool {
    name.is_empty() || name.starts_with('~') || name == class_name || name.starts_with("operator")
}

/// CV_fldattr_t mprop field (bits 4:2 of attr).
/// 0=vanilla 1=virtual 2=static 3=friend 4=intro_virtual 5=pure 6=pure_intro
fn attr_is_virtual(attr: u16) -> bool {
    matches!((attr >> 2) & 7, 1 | 4 | 5 | 6)
}
fn attr_is_static(attr: u16) -> bool {
    (attr >> 2) & 7 == 2
}

fn resolve_method(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    proc_param_index: &ProcParamIndex,
    class_name: &str,
    method_name: &str,
    method_ti: TypeIndex,
    attr: u16,
) -> Option<FunctionInfo> {
    let record = type_stream.record(method_ti).ok()?;
    let td = record.parse().ok()?;

    let mf = match td {
        TypeData::MemberFunc(mf) => mf,
        _ => return None,
    };

    let return_type = resolve_type_name(type_stream, mf.return_value.get(), 0);
    let mut params = extract_params(type_stream, mf.arg_list.get());
    let is_const = inspect_this_const(type_stream, mf.this.get());

    let decorated_names = lookup_decorated_names(sym_index, class_name, method_name);
    let mut decorated_name = decorated_names.first().cloned().unwrap_or_default();

    if let Some((matched_decorated_name, param_names)) =
        choose_best_public_match(&params, decorated_names, proc_param_index)
    {
        decorated_name = matched_decorated_name;
        apply_param_names(&mut params, &param_names);
    } else if let Some(param_names) =
        choose_best_owner_match(&params, class_name, method_name, proc_param_index)
    {
        apply_param_names(&mut params, &param_names);
    }

    Some(FunctionInfo {
        name: method_name.to_owned(),
        decorated_name,
        return_type,
        params,
        is_static: attr_is_static(attr),
        is_virtual: attr_is_virtual(attr),
        is_const,
    })
}

fn resolve_method_list(
    type_stream: &TypeStream<Vec<u8>>,
    sym_index: &SymbolIndex,
    proc_param_index: &ProcParamIndex,
    class_name: &str,
    method_name: &str,
    method_list_ti: TypeIndex,
) -> Vec<FunctionInfo> {
    let record = match type_stream.record(method_list_ti) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let ml_bytes = match record.parse() {
        Ok(TypeData::MethodList(ml)) => ml.bytes,
        _ => return Vec::new(),
    };

    let mut ml = match MethodList::parse(ml_bytes) {
        Ok(ml) => ml,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    while let Ok(Some(item)) = ml.next() {
        if let Some(info) = resolve_method(
            type_stream,
            sym_index,
            proc_param_index,
            class_name,
            method_name,
            item.ty,
            item.attr,
        ) {
            results.push(info);
        }
    }
    results
}

fn extract_params(type_stream: &TypeStream<Vec<u8>>, arg_list_ti: TypeIndex) -> Vec<ParamInfo> {
    if arg_list_ti.0 < TypeIndex::MIN_BEGIN.0 {
        return Vec::new();
    }

    let record = match type_stream.record(arg_list_ti) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let args = match record.parse() {
        Ok(TypeData::ArgList(al)) => al.args.to_vec(),
        _ => return Vec::new(),
    };

    args.iter()
        .enumerate()
        .map(|(i, ti_le)| ParamInfo {
            name: format!("param{}", i),
            type_name: resolve_type_name(type_stream, ti_le.get(), 0),
        })
        .collect()
}

fn apply_param_names(params: &mut [ParamInfo], param_names: &[String]) {
    for (param, param_name) in params.iter_mut().zip(param_names.iter()) {
        if !param_name.is_empty() {
            param.name = param_name.clone();
        }
    }
}

/// A const method has `this: LF_POINTER → LF_MODIFIER(const) → class_type`.
fn inspect_this_const(type_stream: &TypeStream<Vec<u8>>, this_ti: TypeIndex) -> bool {
    if this_ti.0 < TypeIndex::MIN_BEGIN.0 {
        return false;
    }

    let ptr_record = match type_stream.record(this_ti) {
        Ok(r) => r,
        Err(_) => return false,
    };

    let ptr = match ptr_record.parse() {
        Ok(TypeData::Pointer(p)) => p,
        _ => return false,
    };

    let inner_ti = ptr.fixed.ty.get();
    if inner_ti.0 < TypeIndex::MIN_BEGIN.0 {
        return false;
    }

    let mod_record = match type_stream.record(inner_ti) {
        Ok(r) => r,
        Err(_) => return false,
    };

    match mod_record.parse() {
        Ok(TypeData::Modifier(m)) => m.is_const(),
        _ => false,
    }
}

/// Best-effort size estimate for a type in bytes.
fn size_of_type(type_stream: &TypeStream<Vec<u8>>, ti: TypeIndex) -> u32 {
    if ti.0 < TypeIndex::MIN_BEGIN.0 {
        return primitive_size(ti.0);
    }

    let record = match type_stream.record(ti) {
        Ok(r) => r,
        Err(_) => return 0,
    };

    match record.parse() {
        Ok(TypeData::Struct(s)) => u32::try_from(s.length).unwrap_or(0),
        Ok(TypeData::Pointer(_)) => 8, // x64
        Ok(TypeData::Enum(_)) => 4,
        _ => 0,
    }
}

fn primitive_size(ti: u32) -> u32 {
    let mode = (ti >> 8) & 0xF;
    let base = ti & 0xFF;

    if mode > 0 {
        return 8;
    } // pointer (x64)

    match base {
        0x30 => 1,                                    // bool
        0x10 | 0x20 | 0x60 | 0x61 | 0x68 | 0x69 => 1, // char variants
        0x11 | 0x21 | 0x71 => 2,                      // short, wchar_t
        0x12 | 0x22 | 0x40 | 0x74 | 0x75 => 4,        // int, float
        0x13 | 0x23 | 0x41 | 0x76 | 0x77 => 8,        // int64, double
        _ => 0,
    }
}
