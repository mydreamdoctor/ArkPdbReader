/// Builds a name-to-TypeIndex lookup index from the TPI stream.
///
/// This is the primary performance investment: one sequential pass over all
/// TPI records on first use, storing the results in a HashMap.  Every
/// subsequent lookup is O(1) by lowercase name key — no repeated full-stream
/// scans (which is the core defect in the LLVM backend).
///
/// Only LF_CLASS and LF_STRUCTURE records are indexed (not forward declarations).
/// Forward references share the same name as the full definition but have a
/// different TypeIndex; we always prefer the full definition.

use std::collections::HashMap;
use ms_pdb::tpi::TypeStream;
use ms_pdb::codeview::types::kind::Leaf;
use ms_pdb::codeview::types::records::Struct as CvStruct;
use ms_pdb::codeview::parser::Parser;

use crate::type_name::bstr_to_string;

/// Entry stored in the name index for each known UDT.
#[derive(Debug, Clone)]
pub struct NameEntry {
    /// Raw TypeIndex value (u32) pointing to the full definition record in TPI.
    pub raw_ti: u32,
    /// The canonical (exact case) name as stored in the PDB.
    pub canonical_name: String,
    /// True if this is a forward reference, not a full definition.
    /// Forward refs lack field lists.
    pub is_forward_ref: bool,
}

/// Lowercase-keyed HashMap from class/struct name to its TPI entry.
pub type NameIndex = HashMap<String, NameEntry>;

/// Build the name index by scanning all TPI records once.
///
/// For each LF_CLASS or LF_STRUCTURE record:
///   - Skip unnamed records and lambdas / anonymous types.
///   - If the name was already seen and the previous entry was a forward ref
///     while this one is a full definition, replace it (the full definition
///     always wins).
///   - Store both the exact-case name and a lowercase key for case-insensitive
///     lookup.
///
/// Runtime: O(N) where N = number of TPI records.
/// Typical Ark ASA PDB has ~800k–2M type records; expect ~1–3 s first call.
pub fn build_name_index(type_stream: &TypeStream<Vec<u8>>) -> NameIndex {
    let mut index: NameIndex = HashMap::with_capacity(65536);

    let begin_raw: u32 = type_stream.type_index_begin().into();
    let mut current_raw_ti: u32 = begin_raw;

    for record in type_stream.iter_type_records() {
        let ti = current_raw_ti;
        current_raw_ti += 1;

        // Only UDT types are indexed here.
        if record.kind != Leaf::LF_CLASS
            && record.kind != Leaf::LF_STRUCTURE
            && record.kind != Leaf::LF_INTERFACE
        {
            continue;
        }

        let mut parser = Parser::new(record.data);
        let s = match CvStruct::from_parser(&mut parser) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let name = bstr_to_string(s.name);
        if name.is_empty() {
            continue;
        }

        // Skip compiler-generated / anonymous types.
        if is_noise_name(&name) {
            continue;
        }

        let is_fwd = s.fixed.property.get().forward_ref();
        let key = name.to_lowercase();

        // Prefer full definitions over forward references.
        // If we already have a full definition, never overwrite it.
        match index.get(&key) {
            Some(existing) if !existing.is_forward_ref => {
                // Already have a full definition — skip.
                continue;
            }
            Some(_existing) if is_fwd => {
                // Both old and new are forward refs — keep the first one.
                continue;
            }
            _ => {}
        }

        index.insert(
            key,
            NameEntry {
                raw_ti: ti,
                canonical_name: name,
                is_forward_ref: is_fwd,
            },
        );
    }

    index
}

/// True if `name` should not be included in the class index.
/// Filters out lambdas, anonymous structs, compiler temporaries, etc.
fn is_noise_name(name: &str) -> bool {
    // Lambda closures
    if name.contains("lambda") {
        return true;
    }
    // Anonymous namespaces
    if name.contains("anonymous namespace") {
        return true;
    }
    // Compiler-generated temporaries (backtick-quoted names)
    if name.contains('`') {
        return true;
    }
    // Templates are valid but are large in number; we keep them so that
    // members of e.g. TArray<AActor*> can be resolved. The caller (UE class
    // filter in the generator) will exclude them from the top-level class list.
    false
}

/// Case-insensitive lookup in the name index.
/// Returns `None` if the name is not found or is only a forward reference
/// with no field list.
pub fn lookup_name<'a>(index: &'a NameIndex, name: &str) -> Option<&'a NameEntry> {
    let key = name.to_lowercase();
    index.get(&key)
}

/// True if this class name passes the Unreal Engine top-level class filter
/// used by the ArkSdkGen generator.
///
/// The filter accepts names of the form:
///   [A|U|F|E|T|I][A-Z][rest]   — no templates, no namespaces
///
/// This matches what the LLVM backend's `IsPreferredTopLevelClassName` does.
pub fn is_ue_top_level_class(name: &str) -> bool {
    let mut chars = name.chars();

    let prefix = match chars.next() {
        Some(c) => c,
        None => return false,
    };

    let second = match chars.next() {
        Some(c) => c,
        None => return false,
    };

    if !matches!(prefix, 'A' | 'U' | 'F' | 'E' | 'T' | 'I') {
        return false;
    }

    if !second.is_ascii_uppercase() {
        return false;
    }

    // No templates or nested types.
    !name.contains('<') && !name.contains("::")
}
