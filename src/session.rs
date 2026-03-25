use std::cell::OnceCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;

use ms_pdb::tpi::TypeStream;
use ms_pdb::Pdb;

use crate::symbol_stream::{build_pub_rva_index, build_symbol_index, PubRvaIndex, SymbolIndex};
use crate::type_index::{build_name_index, NameIndex};
use crate::types::{ClassLayout, FunctionInfo};

/// Central state for an open PDB session.
///
/// All fields are `pub` so the FFI functions in `lib.rs` can access them
/// directly.  All data is stored in owned `Vec<u8>` / `String` types —
/// the underlying `Pdb` file handle is dropped after `open()`.
pub struct Session {
    /// All TPI records in an owned buffer; the primary data source.
    pub type_stream: TypeStream<Vec<u8>>,

    /// The IPI stream holds function IDs such as LF_FUNC_ID.
    pub ipi_stream: TypeStream<Vec<u8>>,

    /// Raw bytes of the Global Symbol Stream for the decorated-name index.
    pub gss_data: Vec<u8>,

    /// Section virtual addresses (1-based: section 1 → `section_vaddrs[0]`).
    /// Used to convert segment:offset from public symbols to RVAs.
    pub section_vaddrs: Vec<u32>,

    /// lowercase class name → NameEntry; built lazily on first use.
    pub name_index: OnceCell<NameIndex>,

    /// "class::method" (lowercase) → Vec<decorated_name>; built lazily.
    pub symbol_index: OnceCell<SymbolIndex>,

    /// exact decorated name → RVA; built lazily on first use.
    pub pub_rva_index: OnceCell<PubRvaIndex>,

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
        let ipi_stream = pdb.read_ipi_stream()?;
        let gss_data = pdb.read_gss()?.stream_data;

        // Read section headers to enable segment:offset → RVA conversion.
        // An empty vec is safe; symbol RVA lookups will return 0 for any seg.
        let section_vaddrs: Vec<u32> = pdb
            .section_headers()
            .unwrap_or(&[])
            .iter()
            .map(|sh| sh.virtual_address)
            .collect();

        Ok(Session {
            type_stream,
            ipi_stream,
            gss_data,
            section_vaddrs,
            name_index: OnceCell::new(),
            symbol_index: OnceCell::new(),
            pub_rva_index: OnceCell::new(),
            layout_cache: HashMap::new(),
            function_cache: HashMap::new(),
            last_error_cstr: CString::new("").unwrap(),
        })
    }

    /// Access or initialise the name index.
    pub fn name_index(&self) -> &NameIndex {
        self.name_index
            .get_or_init(|| build_name_index(&self.type_stream))
    }

    /// Access or initialise the symbol (decorated-name) index.
    pub fn symbol_index(&self) -> &SymbolIndex {
        self.symbol_index
            .get_or_init(|| build_symbol_index(&self.gss_data))
    }

    /// Access or initialise the public-symbol RVA index.
    pub fn pub_rva_index(&self) -> &PubRvaIndex {
        self.pub_rva_index
            .get_or_init(|| build_pub_rva_index(&self.gss_data, &self.section_vaddrs))
    }

    /// Store an error for retrieval via `ark_pdb_last_error`.
    pub fn set_error(&mut self, msg: &str) {
        self.last_error_cstr = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
    }
}
