#[cfg(feature = "snapshot")]
#[allow(dead_code)]
#[path = "src/host_ops.rs"]
mod host_ops;

fn main() {
    println!("cargo:rerun-if-changed=js/bootstrap.js");
    println!("cargo:rerun-if-changed=src/host_ops.rs");
    #[cfg(feature = "snapshot")]
    snapshot::create();
}

#[cfg(feature = "snapshot")]
mod snapshot {
    use std::collections::HashSet;
    use std::fmt::Write;
    use std::path::PathBuf;

    use deno_runtime::ops::bootstrap::SnapshotOptions;
    use deno_runtime::snapshot::{create_runtime_snapshot, LazyExtensionFileKind};

    pub fn create() {
        let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let snapshot_path = out_dir.join("BB_SNAPSHOT.bin");
        let output = create_runtime_snapshot(
            snapshot_path,
            SnapshotOptions::default(),
            vec![super::host_ops::bb_host::lazy_init()],
        );
        let consumed: HashSet<&String> = output.consumed_lazy_specifiers.iter().collect();
        let mut js = String::from("pub static RESIDUAL_LAZY_JS: &[(&str, &str)] = &[\n");
        let mut esm = String::from("pub static RESIDUAL_LAZY_ESM: &[(&str, &str)] = &[\n");
        let residual_dir = out_dir.join("residual");
        std::fs::create_dir_all(&residual_dir).unwrap();
        for (index, file) in output.lazy_extension_files.iter().enumerate() {
            if consumed.contains(&file.specifier) {
                continue;
            }
            let source = std::fs::read_to_string(&file.path).unwrap();
            let name: deno_core::ModuleName = file.specifier.clone().into();
            let (code, _) = deno_runtime::transpile::maybe_transpile_source(name, source.into())
                .unwrap_or_else(|error| panic!("transpile {}: {error}", file.specifier));
            let target = residual_dir.join(format!("{index}.js"));
            std::fs::write(&target, code.as_str()).unwrap();
            println!("cargo:rerun-if-changed={}", file.path.display());
            let line = format!(
                "    ({:?}, include_str!({:?})),\n",
                file.specifier,
                target.display().to_string()
            );
            match file.kind {
                LazyExtensionFileKind::Js => js.push_str(&line),
                LazyExtensionFileKind::Esm => esm.push_str(&line),
            }
        }
        js.push_str("];\n");
        esm.push_str("];\n");
        let mut generated = String::new();
        writeln!(generated, "{js}{esm}").unwrap();
        std::fs::write(out_dir.join("residual_lazy.rs"), generated).unwrap();
    }
}
