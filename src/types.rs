/// Shared data types used throughout the library and surfaced via the C FFI.
/// These types mirror the `ClassLayoutInfo` / `ClassFunctionInfo` contract
/// defined in ArkSdkGen's `class_layout.h`.

#[derive(Debug, Clone)]
pub struct MemberInfo {
    pub name: String,
    pub type_name: String,
    pub offset: i32,
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub name: String,
    /// The decorated (mangled) C++ name, e.g. `?GetName@AClass@@QEAA...`.
    /// Used by the generator for unique symbol lookup.
    /// Empty string when the symbol could not be resolved from the PSI table.
    pub decorated_name: String,
    pub return_type: String,
    pub params: Vec<ParamInfo>,
    pub is_static: bool,
    pub is_virtual: bool,
    pub is_const: bool,
}

#[derive(Debug, Clone)]
pub struct ClassLayout {
    pub class_name: String,
    pub base_class_name: String,
    pub total_size: u32,
    pub members: Vec<MemberInfo>,
}
