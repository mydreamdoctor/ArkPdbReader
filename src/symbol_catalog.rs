use std::collections::HashSet;

use ms_pdb::syms::{SymData, SymIter, SymKind};
use ms_pdb::tpi::TypeStream;
use ms_pdb::types::{Leaf, TypeData};
use msvc_demangler::{demangle, DemangleFlags};

use crate::type_name::bstr_to_string;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolEntryKind {
    Class,
    Struct,
    Union,
    Enum,
    GlobalFunction,
    GlobalSymbol,
}

#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub name: String,
    pub kind: SymbolEntryKind,
}

pub fn collect_type_entries(type_stream: &TypeStream<Vec<u8>>) -> Vec<SymbolEntry> {
    let mut entries = Vec::new();

    for record in type_stream.iter_type_records() {
        let kind = match record.kind {
            Leaf::LF_CLASS | Leaf::LF_INTERFACE => SymbolEntryKind::Class,
            Leaf::LF_STRUCTURE => SymbolEntryKind::Struct,
            Leaf::LF_UNION => SymbolEntryKind::Union,
            Leaf::LF_ENUM => SymbolEntryKind::Enum,
            _ => continue,
        };

        let Ok(data) = record.parse() else {
            continue;
        };

        let name = match data {
            TypeData::Struct(s) => bstr_to_string(s.name),
            TypeData::Union(u) => bstr_to_string(u.name),
            TypeData::Enum(e) => bstr_to_string(e.name),
            _ => continue,
        };

        if is_noise_name(&name) {
            continue;
        }

        entries.push(SymbolEntry { name, kind });
    }

    sort_and_dedupe(&mut entries);
    entries
}

pub fn collect_global_function_entries(ipi_stream: &TypeStream<Vec<u8>>) -> Vec<SymbolEntry> {
    let mut entries = Vec::new();

    for record in ipi_stream.iter_type_records() {
        let Ok(data) = record.parse() else {
            continue;
        };

        let TypeData::FuncId(function_id) = data else {
            continue;
        };

        let name = bstr_to_string(function_id.name);
        if is_noise_name(&name) {
            continue;
        }

        entries.push(SymbolEntry {
            name,
            kind: SymbolEntryKind::GlobalFunction,
        });
    }

    sort_and_dedupe(&mut entries);
    entries
}

pub fn collect_public_symbol_entries(gss_data: &[u8]) -> Vec<SymbolEntry> {
    let mut entries = Vec::new();

    for sym in SymIter::new(gss_data) {
        if sym.kind != SymKind::S_PUB32 && sym.kind != SymKind::S_PUB32_ST {
            continue;
        }

        let raw_name = match sym.parse() {
            Ok(SymData::Pub(public_symbol)) => {
                match std::str::from_utf8(public_symbol.name.as_ref()) {
                    Ok(value) => value,
                    Err(_) => continue,
                }
            }
            _ => continue,
        };

        let display_name = format_public_symbol_name(raw_name);
        if is_noise_name(&display_name) {
            continue;
        }

        entries.push(SymbolEntry {
            name: display_name,
            kind: SymbolEntryKind::GlobalSymbol,
        });
    }

    sort_and_dedupe(&mut entries);
    entries
}

pub fn sort_and_dedupe(entries: &mut Vec<SymbolEntry>) {
    let mut seen: HashSet<(String, SymbolEntryKind)> = HashSet::with_capacity(entries.len());
    entries.retain(|entry| seen.insert((entry.name.clone(), entry.kind)));

    entries.sort_unstable_by(|left, right| {
        let left_key = symbol_kind_sort_key(left.kind);
        let right_key = symbol_kind_sort_key(right.kind);
        left_key
            .cmp(&right_key)
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn format_public_symbol_name(raw_name: &str) -> String {
    let raw_name = raw_name.trim();
    if raw_name.is_empty() {
        return String::new();
    }

    if raw_name.starts_with('?') {
        if let Ok(demangled) = demangle(raw_name, DemangleFlags::llvm()) {
            return demangled.trim().to_owned();
        }
    }

    raw_name.to_owned()
}

fn is_noise_name(name: &str) -> bool {
    let trimmed = name.trim();
    trimmed.is_empty()
        || contains_ignore_case(trimmed, "lambda")
        || contains_ignore_case(trimmed, "anonymous namespace")
        || contains_ignore_case(trimmed, "dynamic initializer")
        || trimmed.contains('`')
}

fn contains_ignore_case(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(&needle.to_lowercase())
}

fn symbol_kind_sort_key(kind: SymbolEntryKind) -> u8 {
    match kind {
        SymbolEntryKind::Class => 0,
        SymbolEntryKind::Struct => 1,
        SymbolEntryKind::Union => 2,
        SymbolEntryKind::Enum => 3,
        SymbolEntryKind::GlobalFunction => 4,
        SymbolEntryKind::GlobalSymbol => 5,
    }
}
