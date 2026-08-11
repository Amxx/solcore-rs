use std::{collections::BTreeMap, path::PathBuf};

use hir::{
    anchor::DefId,
    ast::item::{Item, Module},
};
use nameres::{Db as _, LibraryId, ModuleKey, module_id_from_key};
use parser::parse_file_to_hir;
use solcore_hir_ty::{
    ClassId, DerivedClauseKind, Evidence, Pred, Solution, Ty, TyCtor, UserTyCtor, UserTyCtorKind,
    canonical_goal, canonical_goal_with_allowed, solve, trait_env_for_module,
    trait_env_from_module_resolution,
};
use solcore_test_utils::{
    define_frontend_test_db, load_fixture_case, load_main_source, load_reachable_modules,
    repo_root_from_manifest,
};

define_frontend_test_db!(TestDb, solcore_hir_ty);

const ABI_SOURCE: &str = r#"
pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

forall a rep . class a:Generic(rep) {}
forall self . class self:ABIDeriving {}
forall self . class self:ABIAttribs {}
forall decoder decoded . class decoder:ABIDecode(decoded) {}
forall reader . class reader:WordReader {}

data ABIDecoder(ty, reader) = ABIDecoder(reader);
data Reader = Reader;

instance Reader:WordReader {}
instance word:ABIAttribs {}
instance ABIDecoder(word, Reader):ABIDecode(word) {}
"#;

fn class_def<'db>(db: &'db TestDb, module: Module<'db>, name: &str) -> DefId<'db> {
    module
        .items(db)
        .iter()
        .find_map(|item| match item {
            Item::ClassDef(class) if class.def_id_value(db).name(db).as_deref() == Some(name) => {
                Some(class.def_id_value(db))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("class {name}"))
}

fn adt_def<'db>(db: &'db TestDb, module: Module<'db>, name: &str) -> DefId<'db> {
    module
        .items(db)
        .iter()
        .find_map(|item| match item {
            Item::AdtDef(adt) if adt.def_id_value(db).name(db).as_deref() == Some(name) => {
                Some(adt.def_id_value(db))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("data type {name}"))
}

fn adt_ty<'db>(db: &'db TestDb, module: Module<'db>, name: &str, args: Vec<Ty<'db>>) -> Ty<'db> {
    Ty::named(
        db,
        TyCtor::User(UserTyCtor {
            def: adt_def(db, module, name),
            kind: UserTyCtorKind::Adt,
        }),
        args,
    )
}

fn decoder_ty<'db>(
    db: &'db TestDb,
    abi_module: Module<'db>,
    decoded: Ty<'db>,
    reader: Ty<'db>,
) -> Ty<'db> {
    adt_ty(db, abi_module, "ABIDecoder", vec![decoded, reader])
}

#[test]
fn derives_parameterized_abi_evidence_once() {
    let mut db = TestDb::default();
    let key = load_main_source(
        &mut db,
        &format!(
            "{ABI_SOURCE}\n\
             data Box(a) = Box(a);\n"
        ),
    );
    let module_id = module_id_from_key(&db, &key);
    let file = db.module_file(module_id).expect("main source file");
    let module = parse_file_to_hir(&db, file).module(&db);
    let resolution = hir::nameres::resolve_module(&db, module);
    assert!(
        resolution.diagnostics.is_empty(),
        "{:?}",
        resolution.diagnostics
    );
    let env = trait_env_from_module_resolution(&db, module, &resolution);

    let boxed_word = adt_ty(&db, module, "Box", vec![Ty::word(&db)]);
    let attribs = Pred::in_class(
        &db,
        ClassId::User(class_def(&db, module, "ABIAttribs")),
        boxed_word,
        Vec::new(),
    );
    let first_attribs = solve(&db, env, canonical_goal(&db, attribs));
    let Solution::Unique {
        evidence: attribs_evidence,
        ..
    } = &first_attribs
    else {
        panic!("expected derived ABIAttribs evidence");
    };
    assert!(matches!(
        attribs_evidence,
        Evidence::Derived {
            kind: DerivedClauseKind::AbiAttribs { .. },
            sub_evidence,
            ..
        } if matches!(sub_evidence.as_slice(), [Evidence::Instance { .. }])
    ));
    assert_eq!(
        solve(&db, env, canonical_goal(&db, attribs)),
        first_attribs,
        "revisiting the same ADT must not introduce a duplicate derived clause",
    );

    let reader = adt_ty(&db, module, "Reader", Vec::new());
    let decode = Pred::in_class(
        &db,
        ClassId::User(class_def(&db, module, "ABIDecode")),
        decoder_ty(&db, module, boxed_word, reader),
        vec![boxed_word],
    );
    let Solution::Unique { evidence, .. } = solve(&db, env, canonical_goal(&db, decode)) else {
        panic!("expected derived ABIDecode evidence");
    };
    assert!(matches!(
        evidence,
        Evidence::Derived {
            kind: DerivedClauseKind::AbiDecode { .. },
            sub_evidence,
            ..
        } if matches!(sub_evidence.as_slice(), [Evidence::Instance { .. }, Evidence::Instance { .. }])
    ));

    let boxed_bool = adt_ty(&db, module, "Box", vec![Ty::bool(&db)]);
    let unsupported = Pred::in_class(
        &db,
        ClassId::User(class_def(&db, module, "ABIAttribs")),
        boxed_bool,
        Vec::new(),
    );
    assert!(matches!(
        solve(&db, env, canonical_goal(&db, unsupported)),
        Solution::NoSolution
    ));
}

#[test]
fn excludes_recursive_no_generic_and_manual_generic_adts() {
    let mut db = TestDb::default();
    let key = load_main_source(
        &mut db,
        &format!(
            "{ABI_SOURCE}\n\
             pragma no-generic-instance-for Excluded;\n\
             data Eligible = Eligible(word);\n\
             data Excluded = Excluded(word);\n\
             data Manual = Manual(word);\n\
             data Recursive = Recursive(Recursive);\n\
             instance Manual:Generic(word) {{}}\n"
        ),
    );
    let module_id = module_id_from_key(&db, &key);
    let file = db.module_file(module_id).expect("main source file");
    let module = parse_file_to_hir(&db, file).module(&db);
    let resolution = hir::nameres::resolve_module(&db, module);
    let env = trait_env_from_module_resolution(&db, module, &resolution);
    let class = ClassId::User(class_def(&db, module, "ABIAttribs"));

    for (name, expected) in [
        ("Eligible", true),
        ("Excluded", false),
        ("Manual", false),
        ("Recursive", false),
    ] {
        let goal = Pred::in_class(
            &db,
            class,
            adt_ty(&db, module, name, Vec::new()),
            Vec::new(),
        );
        assert_eq!(
            matches!(
                solve(&db, env, canonical_goal(&db, goal)),
                Solution::Unique {
                    evidence: Evidence::Derived {
                        kind: DerivedClauseKind::AbiAttribs { .. },
                        ..
                    },
                    ..
                }
            ),
            expected,
            "unexpected ABI derivation result for {name}",
        );
    }
}

#[test]
fn derives_abi_evidence_for_an_imported_adt_closure() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/solver/derived_abi_imported");
    let repo_root = repo_root_from_manifest(env!("CARGO_MANIFEST_DIR"));
    let mut db = TestDb::default();
    let entry = load_fixture_case(&mut db, &fixture, &repo_root, BTreeMap::new());
    load_reachable_modules(&mut db, entry.clone());

    let main_id = module_id_from_key(&db, &entry);
    let abi_id = module_id_from_key(
        &db,
        &ModuleKey {
            library: LibraryId::Main,
            logical_path: vec!["abi".to_owned()],
        },
    );
    let types_id = module_id_from_key(
        &db,
        &ModuleKey {
            library: LibraryId::Main,
            logical_path: vec!["types".to_owned()],
        },
    );
    let abi = parse_file_to_hir(&db, db.module_file(abi_id).expect("abi source")).module(&db);
    let types = parse_file_to_hir(&db, db.module_file(types_id).expect("types source")).module(&db);
    let boxed_word = adt_ty(&db, types, "Box", vec![Ty::word(&db)]);
    let reader = adt_ty(&db, abi, "Reader", Vec::new());
    let goal = Pred::in_class(
        &db,
        ClassId::User(class_def(&db, abi, "ABIDecode")),
        decoder_ty(&db, abi, boxed_word, reader),
        vec![boxed_word],
    );
    assert!(matches!(
        solve(
            &db,
            trait_env_for_module(&db, main_id),
            canonical_goal(&db, goal),
        ),
        Solution::Unique {
            evidence: Evidence::Derived {
                kind: DerivedClauseKind::AbiDecode { .. },
                ..
            },
            ..
        }
    ));
}

#[test]
fn importing_the_marker_does_not_retroactively_activate_an_imported_adt() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/solver/derived_abi_imported_inactive");
    let repo_root = repo_root_from_manifest(env!("CARGO_MANIFEST_DIR"));
    let mut db = TestDb::default();
    let entry = load_fixture_case(&mut db, &fixture, &repo_root, BTreeMap::new());
    load_reachable_modules(&mut db, entry.clone());

    let main_id = module_id_from_key(&db, &entry);
    let abi_id = module_id_from_key(
        &db,
        &ModuleKey {
            library: LibraryId::Main,
            logical_path: vec!["abi".to_owned()],
        },
    );
    let types_id = module_id_from_key(
        &db,
        &ModuleKey {
            library: LibraryId::Main,
            logical_path: vec!["types".to_owned()],
        },
    );
    let abi = parse_file_to_hir(&db, db.module_file(abi_id).expect("abi source")).module(&db);
    let types = parse_file_to_hir(&db, db.module_file(types_id).expect("types source")).module(&db);
    let boxed_word = adt_ty(&db, types, "Box", vec![Ty::word(&db)]);
    let goal = Pred::in_class(
        &db,
        ClassId::User(class_def(&db, abi, "ABIAttribs")),
        boxed_word,
        Vec::new(),
    );
    assert!(matches!(
        solve(
            &db,
            trait_env_for_module(&db, main_id),
            canonical_goal(&db, goal),
        ),
        Solution::NoSolution
    ));
}

#[test]
fn excludes_contract_local_adts_with_inherited_type_binders() {
    let mut db = TestDb::default();
    let key = load_main_source(
        &mut db,
        &format!(
            "{ABI_SOURCE}\n\
             contract C(t) {{\n\
               data Local(a) = Local(a);\n\
             }}\n"
        ),
    );
    let module_id = module_id_from_key(&db, &key);
    let file = db.module_file(module_id).expect("main source file");
    let module = parse_file_to_hir(&db, file).module(&db);
    let resolution = hir::nameres::resolve_module(&db, module);
    let env = trait_env_from_module_resolution(&db, module, &resolution);
    let local = module
        .items(&db)
        .iter()
        .find_map(|item| {
            let Item::ContractDef(contract) = item else {
                return None;
            };
            contract.items(&db).iter().find_map(|item| match item {
                hir::ast::item::ContractItem::AdtDef(adt)
                    if adt.def_id_value(&db).name(&db).as_deref() == Some("Local") =>
                {
                    Some(adt.def_id_value(&db))
                }
                _ => None,
            })
        })
        .expect("contract-local ADT");
    let local_word = Ty::named(
        &db,
        TyCtor::User(UserTyCtor {
            def: local,
            kind: UserTyCtorKind::Adt,
        }),
        vec![Ty::word(&db)],
    );

    for class in ["Generic", "ABIAttribs"] {
        let args = if class == "Generic" {
            vec![Ty::bound(&db, 0)]
        } else {
            Vec::new()
        };
        let goal = Pred::in_class(
            &db,
            ClassId::User(class_def(&db, module, class)),
            local_word,
            args,
        );
        let goal = if class == "Generic" {
            canonical_goal_with_allowed(&db, goal, vec![0])
        } else {
            canonical_goal(&db, goal)
        };
        assert!(matches!(solve(&db, env, goal), Solution::NoSolution));
    }
}
