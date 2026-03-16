/// Smoke-test for ArkPdbReader against the Ark Survival Ascended PDB.
///
/// Run:
///   cargo run --release --example test_pdb -- /path/to/ArkAscendedServer.pdb [ClassName]

use std::env;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

// Import the public C API from the parent crate.
use ark_pdb_reader::{
    ark_pdb_open, ark_pdb_close,
    ark_pdb_list_class_names, ArkClassNameCallback,
    ark_pdb_find_class_layout,
    ark_pdb_layout_free, ark_pdb_layout_get_base_class,
    ark_pdb_layout_get_total_size, ark_pdb_layout_get_member_count,
    ark_pdb_layout_get_member_name, ark_pdb_layout_get_member_type,
    ark_pdb_layout_get_member_offset,
    ark_pdb_find_class_functions,
    ark_pdb_funclist_free, ark_pdb_funclist_get_count,
    ark_pdb_funclist_get_name, ark_pdb_funclist_get_return_type,
    ark_pdb_funclist_is_virtual, ark_pdb_funclist_is_static, ark_pdb_funclist_is_const,
    ark_pdb_funclist_get_param_count, ark_pdb_funclist_get_param_type,
};

fn cstr(s: &str) -> CString { CString::new(s).unwrap() }
fn read_buf(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: test_pdb <path-to-pdb> [ClassName]");
        std::process::exit(1);
    }

    let pdb_path = cstr(&args[1]);
    let class_query = args.get(2).cloned();

    println!("Opening PDB: {}", args[1]);
    let t0 = std::time::Instant::now();

    let session = unsafe { ark_pdb_open(pdb_path.as_ptr()) };
    if session.is_null() {
        eprintln!("ark_pdb_open failed");
        std::process::exit(1);
    }
    println!("Opened in {:.2}s", t0.elapsed().as_secs_f64());

    // ── Class name enumeration ─────────────────────────────────────────────
    let t1 = std::time::Instant::now();
    let mut class_names: Vec<String> = Vec::new();

    unsafe extern "C" fn collect_name(name: *const c_char, user_data: *mut std::ffi::c_void) -> bool {
        let names = &mut *(user_data as *mut Vec<String>);
        names.push(unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned());
        true
    }

    unsafe {
        ark_pdb_list_class_names(
            session,
            collect_name,
            &mut class_names as *mut Vec<String> as *mut std::ffi::c_void,
        );
    }

    println!(
        "Enumerated {} UE classes in {:.2}s",
        class_names.len(),
        t1.elapsed().as_secs_f64()
    );

    if class_query.is_none() {
        println!("\nFirst 20 class names:");
        for name in class_names.iter().take(20) {
            println!("  {}", name);
        }
        if class_names.len() > 20 { println!("  ..."); }

        unsafe { ark_pdb_close(session) };
        return;
    }

    // ── Layout + functions for a specific class ────────────────────────────
    let class_name = class_query.unwrap();
    let class_cstr = cstr(&class_name);

    println!("\n── Layout of '{}' ──", class_name);
    let t2 = std::time::Instant::now();
    let layout = unsafe { ark_pdb_find_class_layout(session, class_cstr.as_ptr()) };

    if layout.is_null() {
        println!("No layout found");
    } else {
        let mut base_buf = [0u8; 512];
        unsafe { ark_pdb_layout_get_base_class(layout, base_buf.as_mut_ptr() as *mut c_char, 512) };
        let base = read_buf(&base_buf);
        let size = unsafe { ark_pdb_layout_get_total_size(layout) };
        let count = unsafe { ark_pdb_layout_get_member_count(layout) };

        println!(
            "size={} bytes  base={}  members={}  ({}ms)",
            size,
            if base.is_empty() { "none" } else { &base },
            count,
            t2.elapsed().as_millis()
        );

        let mut nbuf = [0u8; 256];
        let mut tbuf = [0u8; 512];
        for i in 0..count {
            unsafe {
                ark_pdb_layout_get_member_name(layout, i, nbuf.as_mut_ptr() as *mut c_char, 256);
                ark_pdb_layout_get_member_type(layout, i, tbuf.as_mut_ptr() as *mut c_char, 512);
            }
            let offset = unsafe { ark_pdb_layout_get_member_offset(layout, i) };
            println!("  +{:5}  {:40}  {}", offset, read_buf(&tbuf), read_buf(&nbuf));
        }

        unsafe { ark_pdb_layout_free(layout) };
    }

    println!("\n── Functions of '{}' ──", class_name);
    let t3 = std::time::Instant::now();
    let funcs = unsafe { ark_pdb_find_class_functions(session, class_cstr.as_ptr()) };

    if funcs.is_null() {
        println!("No functions found (or class not found)");
    } else {
        let count = unsafe { ark_pdb_funclist_get_count(funcs) };
        println!("{} functions  ({}ms)", count, t3.elapsed().as_millis());

        let mut nbuf = [0u8; 256];
        let mut rbuf = [0u8; 512];
        let mut pbuf = [0u8; 512];
        for i in 0..count {
            unsafe {
                ark_pdb_funclist_get_name(funcs, i, nbuf.as_mut_ptr() as *mut c_char, 256);
                ark_pdb_funclist_get_return_type(funcs, i, rbuf.as_mut_ptr() as *mut c_char, 512);
            }
            let is_v = unsafe { ark_pdb_funclist_is_virtual(funcs, i) };
            let is_s = unsafe { ark_pdb_funclist_is_static(funcs, i) };
            let is_c = unsafe { ark_pdb_funclist_is_const(funcs, i) };
            let nparams = unsafe { ark_pdb_funclist_get_param_count(funcs, i) };

            let mut param_types: Vec<String> = Vec::new();
            for j in 0..nparams {
                unsafe { ark_pdb_funclist_get_param_type(funcs, i, j, pbuf.as_mut_ptr() as *mut c_char, 512) };
                param_types.push(read_buf(&pbuf));
            }

            let flags = format!(
                "{}{}{}",
                if is_v { "V" } else { " " },
                if is_s { "S" } else { " " },
                if is_c { "C" } else { " " },
            );
            println!(
                "  [{}] {} {}({}) -> {}",
                flags,
                read_buf(&nbuf),
                param_types.join(", "),
                "",
                read_buf(&rbuf),
            );
        }

        unsafe { ark_pdb_funclist_free(funcs) };
    }

    unsafe { ark_pdb_close(session) };
    println!("\nTotal: {:.2}s", t0.elapsed().as_secs_f64());
}
