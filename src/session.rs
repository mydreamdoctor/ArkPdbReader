use std::cell::OnceCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;

use ms_pdb::Pdb;
use ms_pdb::tpi::TypeStream;

use crate::type_index::{NameIndex, build_name_index};
use crate::symbol_stream::{SymbolIndex, build_symbol_index};
use crate::types::{ClassLayout, FunctionInfo};

/// Central state for an open PDB session.
///
/// All fields are `pub` so the FFI functions in `lib.rs` can access them
/// directly.  All data is stored in owned `Vec<u8>` / `String` types —
/// the underlying `Pdb` file handle is dropped after `open()`.
pub struct Session {
    /// All TPI records in an owned buffer; the primary data source.
    pub type_stream: TypeStream<Vec<u8>>,

    /// Raw bytes of the Global Symbol Stream for the decorated-name index.
    pub gss_data: Vec<u8>,

    /// lowercase class name → NameEntry; built lazily on first use.
    pub name_index: OnceCell<NameIndex>,

    /// "class::method" (lowercase) → Vec<decorated_name>; built lazily.
    pub symbol_index: OnceCell<SymbolIndex>,

    /// Per-class layout cache (exact-case key).
    pub layout_cache: HashMap<String, ClassLayout>,

    /// Per-class function cache (exact-case key).
    pub function_cache: HashMap<String, Vec<FunctionInfo>>,

    /// Last error as a null-terminated C string, stable pointer for FFI.
    pub last_error_cstr: CString,
}

impl Session {
    /// Open a PDB file and load all streams into memory.
    ///
    /// After this call all queries are in-memory; the PDB file handle is
    /// closed (the `Pdb` is dropped).
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let pdb = Pdb::open(Path::new(path))?;
        let type_stream = pdb.read_type_stream()?;
        let gss_data = pdb.read_gss()?.stream_data;

        Ok(Session {
            type_stream,
            gss_data,
            name_index: OnceCell::new(),
            symbol_index: OnceCell::new(),
            layout_cache: HashMap::new(),
            function_cache: HashMap::new(),
            last_error_cstr: CString::new("").unwrap(),
        })
    }

    /// Access or initialise the name index.
    pub fn name_index(&self) -> &NameIndex {
        self.name_index.get_or_init(|| build_name_index(&self.type_stream))
    }

    /// Access or initialise the symbol (decorated-name) index.
    pub fn symbol_index(&self) -> &SymbolIndex {
        self.symbol_index.get_or_init(|| build_symbol_index(&self.gss_data))
    }

    /// Store an error for retrieval via `ark_pdb_last_error`.
    pub fn set_error(&mut self, msg: &str) {
        self.last_error_cstr = CString::new(msg)
            .unwrap_or_else(|_| CString::new("error").unwrap());
    }
}
