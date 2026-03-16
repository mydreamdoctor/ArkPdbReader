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
/// directly without a separate accessor layer.
pub struct Session {
    /// Keeps the PDB file handle open for forward compatibility.
    /// Not required for current query operations (all data is read on open),
    /// but retained so future DBI module-symbol stream reads can be added
    /// without an API change.
    _pdb: Pdb<ms_pdb::RandomAccessFile>,

    /// All TPI (Type Information) records, loaded into an owned Vec<u8>.
    /// This is the primary data source for member and method extraction.
    pub type_stream: TypeStream<Vec<u8>>,

    /// Raw bytes of the Global Symbol Stream (GSS), used to build the
    /// decorated-name index on first function query.
    pub gss_data: Vec<u8>,

    // -- Lazy indexes --------------------------------------------------------

    /// lowercase class name → NameEntry (TypeIndex + canonical name).
    /// Built on the first call to `list_class_names` or `type_exists`.
    pub name_index: OnceCell<NameIndex>,

    /// "classname::methodname" (lowercase) → Vec<decorated_name>.
    /// Built on the first call to `find_class_functions`.
    pub symbol_index: OnceCell<SymbolIndex>,

    // -- Per-class memoisation -----------------------------------------------

    /// Layout results keyed by exact-case class name.
    pub layout_cache: HashMap<String, ClassLayout>,

    /// Function list results keyed by exact-case class name.
    pub function_cache: HashMap<String, Vec<FunctionInfo>>,

    // -- Error state ---------------------------------------------------------

    /// Last error as a null-terminated C string, returned by
    /// `ark_pdb_last_error`.  Stored as `CString` so the pointer is stable.
    pub last_error_cstr: CString,
}

impl Session {
    /// Open a PDB file and load all streams needed for queries.
    ///
    /// This is the only I/O-heavy step.  After it returns, all queries use
    /// only in-memory data.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let pdb = Pdb::open(Path::new(path))?;
        let type_stream = pdb.read_type_stream()?;
        let gss = pdb.read_gss()?;
        let gss_data = gss.stream_data;

        Ok(Session {
            _pdb: pdb,
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

    /// Access or initialise the symbol index.
    pub fn symbol_index(&self) -> &SymbolIndex {
        self.symbol_index.get_or_init(|| build_symbol_index(&self.gss_data))
    }

    /// Store an error message for retrieval via `ark_pdb_last_error`.
    pub fn set_error(&mut self, msg: &str) {
        self.last_error_cstr = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
    }
}
