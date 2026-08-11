use std::{collections::BTreeMap, path::PathBuf};

use hir::diag::Diagnostic;
use nameres::{ModuleKey, module_id_from_key};
use solcore_test_utils::{
    define_frontend_test_db, load_fixture_case, load_reachable_modules, lower_any_diagnostics,
    render_diagnostics, repo_root_from_manifest, run_in_large_stack,
};

define_frontend_test_db!(TestDb, solcore_hir_ty);

fn assert_storage_frontend_case(case: &'static str, should_pass: bool, require_clean: bool) {
    run_in_large_stack(move || {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/derived_storage_frontend")
            .join(case);
        let repo_root = repo_root_from_manifest(env!("CARGO_MANIFEST_DIR"));
        let mut db = TestDb::default();
        let entry = load_fixture_case(&mut db, &fixture, &repo_root, BTreeMap::new());
        load_reachable_modules(&mut db, entry.clone());
        let diagnostics = full_frontend_diagnostics(&db, entry);
        let has_storage_failure = diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("CanStore") || diagnostic.message.contains("StorageSize")
        });
        assert_eq!(
            !has_storage_failure,
            should_pass,
            "unexpected storage frontend result for {case}\n{}",
            render_diagnostics(&db, &diagnostics),
        );
        if require_clean {
            assert!(
                diagnostics.is_empty(),
                "positive storage field validation emitted diagnostics for {case}\n{}",
                render_diagnostics(&db, &diagnostics),
            );
        }
    });
}

#[test]
fn exact_mapping_valued_adt_field_is_rejected_even_when_unused() {
    assert_storage_frontend_case("storage-adt-mapping-field-fail", false, false);
}

#[test]
fn exact_recursive_adt_field_is_rejected() {
    assert_storage_frontend_case("storage-adt-recursive-fail", false, false);
}

#[test]
fn exact_recursive_adt_remains_usable_outside_storage() {
    assert_storage_frontend_case("storage-adt-recursive-ok", true, false);
}

#[test]
fn unused_builtin_storage_fields_keep_their_loaded_types() {
    assert_storage_frontend_case("storage-builtins-unused-ok", true, true);
}

#[test]
fn imported_storage_adt_needs_no_consumer_deriving_imports() {
    assert_storage_frontend_case("storage-imported-active-no-marker-ok", true, true);
}

#[test]
fn body_only_imported_storage_adt_uses_definition_side_evidence() {
    assert_storage_frontend_case("storage-body-only-active-ok", true, true);
}

#[test]
fn qualified_body_only_import_uses_definition_side_evidence() {
    assert_storage_frontend_case("storage-body-only-qualified-ok", true, true);
}

#[test]
fn body_only_import_does_not_retroactively_enable_storage_derivation() {
    assert_storage_frontend_case("storage-body-only-inactive-fail", false, false);
}

#[test]
fn nested_reexport_does_not_leak_definition_side_storage_evidence() {
    assert_storage_frontend_case("storage-nested-reexport-invalid-fail", false, false);
}

#[test]
fn imported_mapping_valued_adt_is_rejected_without_consumer_marker() {
    assert_storage_frontend_case("storage-imported-invalid-no-marker-fail", false, false);
}

#[test]
fn consumer_does_not_retroactively_enable_imported_storage_adt() {
    assert_storage_frontend_case("storage-imported-inactive-no-marker-fail", false, false);
}

fn full_frontend_diagnostics(db: &TestDb, entry: ModuleKey) -> Vec<Diagnostic> {
    let entry = module_id_from_key(db, &entry);
    let mut diagnostics = nameres::reachable_diagnostics(db, entry).to_vec();
    diagnostics.extend(
        solcore_hir_ty::infer::reachable_typeck_diagnostics(db, entry)
            .iter()
            .cloned(),
    );
    lower_any_diagnostics(db, diagnostics)
}
