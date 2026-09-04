//! Compiles every playground example source through the full driver pipeline
//! so the examples cannot drift out of sync with the compiler. Entry files
//! (those declaring a contract) additionally go through Hull emission, which
//! catches constructs the frontend accepts but the backends cannot lower.

use std::{fs, path::PathBuf, process::Command};

#[test]
fn playground_examples_compile() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let examples_root = root.join("playground/src/examples");
    let std_root = root.join("std");

    let mut checked = 0;
    let mut failures = Vec::new();

    let mut dirs: Vec<_> = fs::read_dir(&examples_root)
        .expect("playground examples directory")
        .filter_map(|entry| {
            let path = entry.expect("readable directory entry").path();
            path.is_dir().then_some(path)
        })
        .collect();
    dirs.sort();

    for dir in dirs {
        let mut files: Vec<_> = fs::read_dir(&dir)
            .expect("readable example directory")
            .filter_map(|entry| {
                let path = entry.expect("readable file entry").path();
                (path.extension().is_some_and(|ext| ext == "sol")).then_some(path)
            })
            .collect();
        files.sort();

        for file in files {
            let source = fs::read_to_string(&file).expect("readable example source");
            let has_contract = source.lines().any(|line| line.starts_with("contract "));

            let mut command = Command::new(env!("CARGO_BIN_EXE_solcore-driver"));
            command.arg("--std-root").arg(&std_root);
            if has_contract {
                command.arg("--emit-hull=/dev/null");
            }
            let output = command.arg(&file).output().expect("driver invocation");

            if !output.status.success() || !output.stderr.is_empty() {
                failures.push(format!(
                    "{}:\n{}",
                    file.display(),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            checked += 1;
        }
    }

    assert!(
        checked >= 16,
        "expected playground example sources, found {checked}"
    );
    assert!(
        failures.is_empty(),
        "playground examples failed to compile:\n{}",
        failures.join("\n")
    );
}
