// bukio-cli — embed the SQL migrations into the binary at compile time.
// (mirrors src/core/db.js reading migrations/ at runtime; embedding removes
// all path-resolution issues for installed binaries)
use std::{env, fs, path::Path};

fn main() {
    let dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let migrations = Path::new(&dir).join("migrations");
    let mut entries: Vec<(u32, String)> = fs::read_dir(&migrations)
        .expect("migrations dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| {
            let mut cs = n.chars();
            cs.next().map_or(false, |c| c.is_ascii_digit()) && n.ends_with(".sql")
        })
        .map(|n| {
            let v: u32 = n.split('_').next().unwrap().parse().unwrap();
            (v, n)
        })
        .collect();
    entries.sort();
    let mut out = String::from("pub static MIGRATIONS: &[(u32, &str)] = &[\n");
    for (v, n) in entries {
        println!("cargo:rerun-if-changed=migrations/{n}");
        out.push_str(&format!(
            "    ({v}, include_str!(\"../migrations/{n}\")),\n"
        ));
    }
    out.push_str("];\n");
    let dest = Path::new(&dir).join("src").join("migrations_data.rs");
    // only rewrite when changed, so cargo doesn't needlessly rebuild
    if !matches!(fs::read_to_string(&dest), Ok(ref s) if *s == out) {
        fs::write(&dest, out).unwrap();
    }
}
