/// Builds a decorated-name index from the PDB Public Symbol Index (PSI).
///
/// Decorated (mangled) C++ names look like:
///   `?GetPlayerName@APlayerController@@QEAA?AVFString@@XZ`
///
/// For member functions, MSVC mangling follows the pattern:
///   `?<MethodName>@<ClassName>@@<qualifiers>...`
///
/// We parse this cheaply — no full demangler needed — to build the mapping:
///   lowercase "ClassName::MethodName"  →  Vec<decorated_name>
///
/// When the generator calls `find_class_functions`, we look up each method's
/// decorated name from this index by its owner class and short name.
///
/// Limitations:
///   - Overloaded methods produce multiple entries in the Vec.
///     The caller receives all candidates and should pick by signature.
///   - Nested class methods (Outer::Inner::Method) only extract the
///     innermost class name and may be ambiguous.
///   - Static methods, free functions, and non-MSVC-mangled names are skipped.

use std::collections::HashMap;
use ms_pdb::syms::{SymData, SymIter, SymKind};

/// Map from lowercase "ClassName::MethodName" to one or more decorated names.
pub type SymbolIndex = HashMap<String, Vec<String>>;

/// Build the symbol index by scanning the Global Symbol Stream (GSS).
///
/// The GSS is a flat byte sequence of symbol records.  We iterate public
/// symbols (S_PUB32), extract the decorated name, parse the owner class and
/// method name, and store them in the index.
///
/// Runtime: O(N) where N is the number of public symbols.
pub fn build_symbol_index(gss_data: &[u8]) -> SymbolIndex {
    let mut index: SymbolIndex = HashMap::with_capacity(32768);

    for sym in SymIter::new(gss_data) {
        if sym.kind != SymKind::S_PUB32 && sym.kind != SymKind::S_PUB32_ST {
            continue;
        }

        let decorated = match sym.parse() {
            Ok(SymData::Pub(p)) => match std::str::from_utf8(p.name.as_ref()) {
                Ok(s) => s,
                Err(_) => continue,
            },
            _ => continue,
        };

        // Only index MSVC-mangled C++ names — they start with '?'.
        if !decorated.starts_with('?') {
            continue;
        }

        // Parse "?MethodName@ClassName@@..." to extract class and method.
        if let Some((class_name, method_name)) = parse_msvc_mangled_owner(&decorated) {
            let key = format!(
                "{}::{}",
                class_name.to_lowercase(),
                method_name.to_lowercase()
            );
            index
                .entry(key)
                .or_default()
                .push(decorated.to_owned());
        }
    }

    index
}

/// Parse an MSVC-mangled symbol name to extract (ClassName, MethodName).
///
/// MSVC mangling for a member function looks like:
///   `?MethodName@ClassName@@qualifiers...`
///
/// Rules applied:
///   - Must start with `?`
///   - The first segment (between `?` and the first `@`) is the method name.
///   - The second segment (between first `@` and the `@@` terminator) is the
///     class name.
///   - If the class name contains `@` (nested class), we take the last
///     component (innermost class).
///
/// Returns `None` for global functions or names that don't match the pattern.
fn parse_msvc_mangled_owner(mangled: &str) -> Option<(&str, &str)> {
    // Must start with ?
    let rest = mangled.strip_prefix('?')?;

    // Method name: up to the first @
    let at_pos = rest.find('@')?;
    let method_name = &rest[..at_pos];

    // The class specifier follows: everything up to `@@`
    let after_method = &rest[at_pos + 1..];
    let end_of_class = after_method.find("@@")?;
    let class_spec = &after_method[..end_of_class];

    // class_spec may be "ClassName" or "InnerClass@OuterClass" for nested.
    // Take the first component (innermost class).
    let class_name = class_spec.split('@').next()?;

    if class_name.is_empty() || method_name.is_empty() {
        return None;
    }

    Some((class_name, method_name))
}

/// Look up all decorated names for a given class and method name.
/// Returns an empty slice if nothing is found.
pub fn lookup_decorated_names<'a>(
    index: &'a SymbolIndex,
    class_name: &str,
    method_name: &str,
) -> &'a [String] {
    let key = format!(
        "{}::{}",
        class_name.to_lowercase(),
        method_name.to_lowercase()
    );
    index.get(&key).map(Vec::as_slice).unwrap_or(&[])
}
