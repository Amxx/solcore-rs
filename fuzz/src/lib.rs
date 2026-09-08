//! Small, in-process entrypoints used by the AFL++ binaries in `src/bin`.
//!
//! Expected source diagnostics return normally.  A panic, abort, or timeout is
//! therefore the signal recorded by AFL++ as a finding.

use std::{env, panic, path::Path, thread};

use hir::{diag::DiagnosticLevel, input::SourceFile};
use parser::{parse_diagnostics, parse_file_to_hir};
use vfs::Workspace;

/// Largest source accepted by the initial byte-stream targets.
///
/// Larger generated programs belong in a structured multi-file target, where
/// their size can be budgeted independently from parser fuzzing throughput.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// Match the native driver's compiler-thread stack so a deep valid input does
/// not become a harness-only stack-overflow finding.
pub const COMPILER_STACK_SIZE: usize = 256 * 1024 * 1024;
/// Number of inputs processed before AFL++ replaces the persistent child.
///
/// Compiler databases and parser graphs are reclaimed after every input.
/// Recycling the child still bounds the impact of allocator/thread-local high
/// water and future dependency regressions without paying for a fresh process
/// on every input.
pub const DEFAULT_PERSISTENT_LOOP_COUNT: usize = 10;

const AFL_FUZZER_LOOPCOUNT: &str = "AFL_FUZZER_LOOPCOUNT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Parser,
    Frontend,
    Backend,
}

/// Runs an AFL++ target with a bounded persistent-child lifetime.
///
/// Operators can override [`DEFAULT_PERSISTENT_LOOP_COUNT`] by setting
/// `AFL_FUZZER_LOOPCOUNT` before starting the target.
///
/// # Safety
///
/// Call this at the start of `main`, before any other threads are created or
/// any foreign code can concurrently access the process environment.
pub unsafe fn fuzz(target: Target) {
    // SAFETY: The caller guarantees exclusive access to the process
    // environment until this function has configured the persistent loop.
    unsafe { configure_persistent_loop() };

    let fuzzer = thread::Builder::new()
        .name(format!("solcore-{target:?}-fuzz").to_ascii_lowercase())
        .stack_size(COMPILER_STACK_SIZE)
        .spawn(move || afl::fuzz!(|input: &[u8]| process(target, input)))
        .expect("failed to spawn compiler fuzzing thread");
    if let Err(payload) = fuzzer.join() {
        panic::resume_unwind(payload);
    }
}

unsafe fn configure_persistent_loop() {
    // Match afl.rs, which reads this setting with `env::var`. A non-UTF-8 value
    // is unusable there, so treat it as absent instead of accidentally leaving
    // afl.rs with its effectively unbounded fallback.
    apply_persistent_loop_default(env::var(AFL_FUZZER_LOOPCOUNT).ok(), |name, value| {
        // SAFETY: The caller of `configure_persistent_loop` guarantees that no
        // other thread can access the process environment concurrently.
        unsafe { env::set_var(name, value) };
    });
}

fn apply_persistent_loop_default(
    configured: Option<String>,
    mut set_var: impl FnMut(&str, String),
) {
    if configured.is_none() {
        set_var(
            AFL_FUZZER_LOOPCOUNT,
            DEFAULT_PERSISTENT_LOOP_COUNT.to_string(),
        );
    }
}

/// Runs one input through a fuzz target.
///
/// Inputs that are not UTF-8 and ordinary compiler rejections are intentionally
/// ignored.  The native compiler's source transport is UTF-8, and the target
/// property is absence of crashes while reporting a diagnostic.
pub fn process(target: Target, input: &[u8]) {
    if input.len() > MAX_INPUT_BYTES {
        return;
    }
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    match target {
        Target::Parser => parse(source),
        Target::Frontend => frontend(source),
        Target::Backend => backend(source),
    }
}

fn parse(source: &str) {
    let db = ParserDb::default();
    let file = source_file(&db, source);
    let _ = parse_file_to_hir(&db, file).module(&db);
    let _ = parse_diagnostics(&db, file);
}

fn frontend(source: &str) {
    let workspace = workspace_with_entry(source);
    let _ = workspace.raw_diagnostics();
}

fn backend(source: &str) {
    let workspace = workspace_with_entry(source);
    let diagnostics = workspace.raw_diagnostics();
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
    {
        return;
    }
    drop(diagnostics);

    let entry_file = workspace
        .db()
        .source_file(Path::new(vfs::MAIN_ROOT).join("main.sol"))
        .expect("fuzz entry file was inserted into the VFS");
    let _ = compiler::build_checked_hull(
        workspace.db(),
        entry_file,
        specialize::SpecializeOptions::default(),
    );
}

fn workspace_with_entry(source: &str) -> Workspace {
    let mut workspace = Workspace::new();
    workspace.set_file("main.sol", source.to_owned());
    workspace.set_entry("main.sol");
    workspace
}

#[salsa::db]
#[derive(Default)]
struct ParserDb {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for ParserDb {}

#[salsa::db]
impl hir::Db for ParserDb {
    fn def_location_table<'db>(
        &'db self,
        file: SourceFile,
    ) -> &'db hir::anchor::DefLocationTable<'db> {
        parse_file_to_hir(self, file).def_locations(self)
    }
}

#[salsa::db]
impl parser::Db for ParserDb {}

fn source_file(db: &ParserDb, source: &str) -> SourceFile {
    let url = url::Url::parse("memory:///fuzz/main.sol").expect("constant URL is valid");
    SourceFile::new(db, url, Some(source.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEPTED: &[u8] = b"function id(x: word) returns (word) { return x; }\n";
    const REJECTED: &[u8] = b"function main() returns (word) { return true; }\n";

    #[test]
    fn every_target_accepts_compiler_diagnostics_normally() {
        for target in [Target::Parser, Target::Frontend, Target::Backend] {
            process(target, ACCEPTED);
            process(target, REJECTED);
        }
    }

    #[test]
    fn non_utf8_and_oversized_inputs_are_skipped() {
        process(Target::Frontend, &[0xff]);
        process(Target::Frontend, &vec![b'x'; MAX_INPUT_BYTES + 1]);
    }

    #[test]
    fn persistent_loop_is_bounded_by_default() {
        let mut configured = None;
        apply_persistent_loop_default(None, |name, value| {
            configured = Some((name.to_owned(), value));
        });

        assert_eq!(
            configured,
            Some((
                AFL_FUZZER_LOOPCOUNT.to_owned(),
                DEFAULT_PERSISTENT_LOOP_COUNT.to_string(),
            ))
        );
    }

    #[test]
    fn explicit_persistent_loop_count_is_preserved() {
        let mut configured = None;
        apply_persistent_loop_default(Some("37".to_owned()), |name, value| {
            configured = Some((name.to_owned(), value));
        });

        assert_eq!(configured, None);
    }
}
