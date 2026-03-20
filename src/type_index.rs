/// Builds a name-to-TypeIndex lookup index from the TPI stream.
///
/// One sequential TPI pass on first use; all subsequent lookups are O(1).

use std::collections::HashMap;
use ms_pdb::tpi::TypeStream;
use ms_pdb::types::{Leaf, TypeData, TypeIndex};

use crate::type_name::bstr_to_string;

/// Lightweight kind carried in the cached UDT name index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Class,
    Struct,
}

/// Entry stored in the name index for each known UDT.
#[derive(Debug, Clone)]
pub struct NameEntry {
    /// Raw TypeIndex pointing to the definition record in TPI.
    pub type_index: TypeIndex,
    /// The canonical (exact case) name as stored in the PDB.
    pub canonical_name: String,
    /// Whether this UDT was declared as a class or struct in TPI.
    pub kind: TypeKind,
    /// True if this entry is a forward reference (incomplete definition).
    pub is_forward_ref: bool,
}

/// Lowercase-keyed map from class/struct name to its TPI entry.
pub type NameIndex = HashMap<String, NameEntry>;

/// Build the name index by scanning all TPI records once.
///
/// Prefers full definitions over forward references.
/// Runtime: O(N) in the number of TPI records.
pub fn build_name_index(type_stream: &TypeStream<Vec<u8>>) -> NameIndex {
    let mut index: NameIndex = HashMap::with_capacity(65536);

    let mut current_ti = type_stream.type_index_begin().0;

    for record in type_stream.iter_type_records() {
        let ti = TypeIndex(current_ti);
        current_ti += 1;

        let kind = match record.kind {
            Leaf::LF_CLASS | Leaf::LF_INTERFACE => TypeKind::Class,
            Leaf::LF_STRUCTURE => TypeKind::Struct,
            _ => continue,
        };

        let td = match record.parse() {
            Ok(td) => td,
            Err(_) => continue,
        };

        let (name, is_fwd) = match td {
            TypeData::Struct(s) => {
                let n = bstr_to_string(s.name);
                let fwd = s.fixed.property.get().fwdref();
                (n, fwd)
            }
            _ => continue,
        };

        if name.is_empty() || is_noise_name(&name) {
            continue;
        }

        let key = name.to_lowercase();

        // Full definitions always win over forward references.
        match index.get(&key) {
            Some(e) if !e.is_forward_ref => continue, // already have full def
            Some(_) if is_fwd => continue,             // both fwd, keep first
            _ => {}
        }

        index.insert(key, NameEntry { type_index: ti, canonical_name: name, kind, is_forward_ref: is_fwd });
    }

    index
}

fn is_noise_name(name: &str) -> bool {
    name.contains("lambda") || name.contains("anonymous namespace") || name.contains('`')
}

/// Case-insensitive lookup in the name index.
pub fn lookup_name<'a>(index: &'a NameIndex, name: &str) -> Option<&'a NameEntry> {
    index.get(&name.to_lowercase())
}

/// True if this class name passes the Unreal Engine top-level class filter
/// used by the ArkSdkGen generator: [A|U|F|E|T|I][A-Z][rest], no templates,
/// no namespaces.
pub fn is_ue_top_level_class(name: &str) -> bool {
    let mut chars = name.chars();
    let prefix = match chars.next() { Some(c) => c, None => return false };
    let second = match chars.next() { Some(c) => c, None => return false };
    if !matches!(prefix, 'A' | 'U' | 'F' | 'E' | 'T' | 'I') { return false; }
    if !second.is_ascii_uppercase() { return false; }
    !name.contains('<') && !name.contains("::")
}
