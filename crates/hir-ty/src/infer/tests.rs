use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use hir::{
    anchor::{DefId, DefLocationTable},
    ast::{
        function::{ExprKind, FuncParam, FuncSig, StmtKind},
        item::{ContractItem, FunctionDef, Item, Module},
    },
    input::SourceFile,
    nameres::{self as hir_nameres, ident_text, type_var_bindings},
    sema::ty::QualTy,
};
use nameres::{
    LibraryId, ModuleFileSnapshot, ModuleFsSnapshot, ModuleId, ModuleKey, ModuleTree,
    module_id_from_key, module_key_for_path,
};
use parser::parse_file_to_hir;
use salsa::Setter;

use super::*;
use crate::{
    BinderEnv, ClauseOrigin, Solution, TraitEnvId, TypeLowering, UserTyCtor, UserTyCtorKind,
    canonical_goal, solve, solve_report, trait_env_for_module, trait_env_from_module_resolution,
    trait_env_from_module_resolution_and_imports, trait_env_with_givens,
};

#[salsa::db]
#[derive(Default, Clone)]
struct TestDb {
    storage: salsa::Storage<Self>,
    module_file_snapshot: Option<ModuleFileSnapshot>,
    module_files: FxHashMap<ModuleKey, SourceFile>,
}

impl TestDb {
    fn insert_module_file(&mut self, key: ModuleKey, file: SourceFile) {
        if self.module_files.insert(key, file) == Some(file) {
            return;
        }
        let files = self
            .module_files
            .iter()
            .map(|(key, file)| (key.clone(), *file))
            .collect();
        if let Some(snapshot) = self.module_file_snapshot {
            snapshot.set_files(self).to(files);
        } else {
            self.module_file_snapshot = Some(ModuleFileSnapshot::new(self, files));
        }
    }
}

#[salsa::db]
impl salsa::Database for TestDb {}

#[salsa::db]
impl hir::Db for TestDb {
    fn def_location_table<'db>(&'db self, file: SourceFile) -> &'db DefLocationTable<'db> {
        parse_file_to_hir(self, file).def_locations(self)
    }
}

#[salsa::db]
impl parser::Db for TestDb {}

#[salsa::db]
impl nameres::Db for TestDb {
    fn module_tree(&self) -> ModuleTree {
        ModuleTree::new(
            self,
            PathBuf::from("/main"),
            PathBuf::from("/std"),
            BTreeMap::new(),
        )
    }

    fn module_fs_snapshot(&self) -> ModuleFsSnapshot {
        ModuleFsSnapshot::new(self, BTreeSet::new(), BTreeMap::new())
    }

    fn module_file_snapshot(&self) -> ModuleFileSnapshot {
        self.module_file_snapshot
            .unwrap_or_else(|| ModuleFileSnapshot::new(self, BTreeMap::new()))
    }

    fn module_file<'db>(&'db self, module: ModuleId<'db>) -> Option<SourceFile> {
        self.module_file_snapshot()
            .files(self)
            .get(&module.key(self))
            .copied()
    }
}

#[salsa::db]
impl crate::Db for TestDb {}

fn source_file(db: &TestDb, name: &str, src: &str) -> SourceFile {
    let url = format!("memory:///{name}.solc").parse().expect("valid url");
    SourceFile::new(db, url, Some(src.to_owned()))
}

fn source_file_at_path(db: &TestDb, path: &std::path::Path, src: &str) -> SourceFile {
    let url = url::Url::from_file_path(path).expect("file url");
    SourceFile::new(db, url, Some(src.to_owned()))
}

fn parse_module<'db>(db: &'db TestDb, src: &str) -> Module<'db> {
    parse_file_to_hir(db, source_file(db, "hir_ty", src)).module(db)
}

fn module_key(path: &[&str]) -> ModuleKey {
    ModuleKey {
        library: LibraryId::Main,
        logical_path: path.iter().map(|segment| (*segment).to_owned()).collect(),
    }
}

fn insert_module_source(db: &mut TestDb, path: &[&str], src: &str) -> ModuleKey {
    let key = module_key(path);
    let url = format!("memory:///{}.solc", path.join("/"))
        .parse()
        .expect("valid url");
    let file = SourceFile::new(&*db, url, Some(src.to_owned()));
    db.insert_module_file(key.clone(), file);
    key
}

fn db_with_main_typeck(src: &str) -> (TestDb, ModuleKey) {
    let mut db = TestDb::default();
    let key = insert_module_source(&mut db, &["main"], src);
    (db, key)
}

fn db_with_array_std(main_src: &str) -> (TestDb, ModuleKey) {
    let mut db = TestDb::default();
    let std_path = PathBuf::from("/std/std.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let std_file = source_file_at_path(
        &db,
        &std_path,
        r#"
export { memory(*), storage(*), DynArray, array(*), uint256(*), address(*), string, concatLit, Add, Typedef, CanStore };

data memory(t) = memory(word);
data storage(t) = storage(word);
data DynArray(t);
data array(t) = array(word);
data uint256 = uint256(word);
data address = address(word);
data string;

function concatLit(comptime lhs:string, comptime rhs:string) -> string { return lhs; }

forall self . class self:Add {
  function add(lhs:self, rhs:self) -> self;
}

instance uint256:Add {
  function add(lhs:uint256, rhs:uint256) -> uint256 { return lhs; }
}

forall abs rep . class abs:Typedef(rep) {
  function abs(x:rep) -> abs;
  function rep(x:abs) -> rep;
}

forall t . default instance t:Typedef(t) {
  function abs(x:t) -> t { return x; }
  function rep(x:t) -> t { return x; }
}

instance uint256:Typedef(word) {
  function abs(x:word) -> uint256 { return uint256(x); }
  function rep(x:uint256) -> word { return 0; }
}

instance memory(string):Typedef(word) {
  function abs(x:word) -> memory(string) { return memory(x); }
  function rep(x:memory(string)) -> word { return 0; }
}

forall dst value . class dst:CanStore(value) {
  function store(dst:dst, value:value) -> ();
  function load(dst:dst) -> value;
}

instance storage(word):CanStore(word) {
  function store(dst:storage(word), value:word) -> () { return (); }
  function load(dst:storage(word)) -> word { return 0; }
}

instance storage(uint256):CanStore(uint256) {
  function store(dst:storage(uint256), value:uint256) -> () { return (); }
  function load(dst:storage(uint256)) -> uint256 { return uint256(0); }
}

instance storage(string):CanStore(memory(string)) {
  function store(dst:storage(string), value:memory(string)) -> () { return (); }
  function load(dst:storage(string)) -> memory(string) { return memory(0); }
}

instance storage(array(word)):CanStore(storage(array(word))) {
  function store(dst:storage(array(word)), value:storage(array(word))) -> () { return (); }
  function load(dst:storage(array(word))) -> storage(array(word)) { return dst; }
}
"#,
    );
    let main_file = source_file_at_path(&db, &main_path, main_src);
    let std_key = module_key_for_path(LibraryId::Std, &PathBuf::from("/std"), &std_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(std_key, std_file);
    db.insert_module_file(main_key.clone(), main_file);
    (db, main_key)
}

fn canonical_std_adt_ty<'db>(db: &'db TestDb, name: &str, args: Vec<Ty<'db>>) -> Ty<'db> {
    let def = crate::support::canonical_std_adt_def(db, name).expect("canonical std type");
    Ty::named(
        db,
        TyCtor::User(UserTyCtor {
            def,
            kind: UserTyCtorKind::Adt,
        }),
        args,
    )
}

fn lowered_module_typeck_diagnostics(src: &str) -> Vec<Diagnostic> {
    let (db, key) = db_with_main_typeck(src);
    let module = module_id_from_key(&db, &key);
    module_typeck_diagnostics(&db, module)
        .iter()
        .map(|diagnostic| diagnostic.lower(&db))
        .collect()
}

fn function_name<'db>(db: &'db TestDb, function: FunctionDef<'db>) -> &'db str {
    (*function.sig(db).name.atom()).text(db)
}

fn sig_type_vars<'db>(
    owner: DefId<'db>,
    sig: &FuncSig<'db>,
) -> Vec<hir_nameres::TypeVarBinding<'db>> {
    type_var_bindings(owner, &sig.type_vars)
}

fn param_names<'db>(db: &'db TestDb, params: &[FuncParam<'db>]) -> Vec<String> {
    params
        .iter()
        .filter_map(|param| match param {
            FuncParam::Typed { name, .. } | FuncParam::Untyped { name, .. } => {
                Some(ident_text(db, name))
            }
            FuncParam::Error { .. } => None,
        })
        .collect()
}

#[derive(Clone)]
struct FunctionInfo<'db> {
    function: FunctionDef<'db>,
    type_vars: Vec<hir_nameres::TypeVarBinding<'db>>,
}

fn function_infos<'db>(db: &'db TestDb, module: Module<'db>) -> Vec<FunctionInfo<'db>> {
    let mut infos = Vec::new();
    for item in module.items(db) {
        collect_function_infos(db, *item, &[], &mut infos);
    }
    infos
}

fn collect_function_infos<'db>(
    db: &'db TestDb,
    item: Item<'db>,
    inherited: &[hir_nameres::TypeVarBinding<'db>],
    infos: &mut Vec<FunctionInfo<'db>>,
) {
    match item {
        Item::FunctionDef(function) => push_function_info(db, function, inherited, infos),
        Item::InstanceDef(instance) => {
            let mut inherited = inherited.to_vec();
            inherited.extend(type_var_bindings(
                instance.def_id_value(db),
                instance.type_var_elems(db),
            ));
            for method in instance.methods(db) {
                push_function_info(db, *method, &inherited, infos);
            }
        }
        Item::ContractDef(contract) => {
            let mut inherited = inherited.to_vec();
            inherited.extend(type_var_bindings(
                contract.def_id_value(db),
                contract.ty_param_elems(db),
            ));
            for item in contract.items(db) {
                match *item {
                    ContractItem::FunctionDef(function) => {
                        push_function_info(db, function, &inherited, infos)
                    }
                    ContractItem::TypeAlias(_)
                    | ContractItem::AdtDef(_)
                    | ContractItem::Error { .. } => {}
                }
            }
        }
        Item::TypeAlias(_)
        | Item::AdtDef(_)
        | Item::ClassDef(_)
        | Item::Import(_)
        | Item::Export(_)
        | Item::Pragma(_)
        | Item::Error { .. } => {}
    }
}

fn push_function_info<'db>(
    db: &'db TestDb,
    function: FunctionDef<'db>,
    inherited: &[hir_nameres::TypeVarBinding<'db>],
    infos: &mut Vec<FunctionInfo<'db>>,
) {
    let mut type_vars = inherited.to_vec();
    type_vars.extend(sig_type_vars(function.def_id_value(db), function.sig(db)));
    infos.push(FunctionInfo {
        function,
        type_vars,
    });
}

fn body_map<'db>(
    db: &'db TestDb,
    module_resolution: &hir_nameres::ModuleResolutionMap<'db>,
    body: FuncBody<'db>,
) -> hir_nameres::BodyResolutionMap<'db> {
    module_resolution
        .bodies
        .iter()
        .find(|map| {
            map.exprs.iter().any(|entry| entry.body == body)
                || map.stmt_bindings.iter().any(|entry| entry.body == body)
                || map.pats.iter().any(|entry| entry.body == body)
        })
        .cloned()
        .unwrap_or_else(|| {
            // Bodies with no resolvable names (e.g. only literals) have no
            // entries to match on; an empty map is the correct fallback.
            let _ = db;
            hir_nameres::BodyResolutionMap::default()
        })
}

fn trait_env<'db>(
    db: &'db TestDb,
    module: Module<'db>,
    module_resolution: &hir_nameres::ModuleResolutionMap<'db>,
) -> TraitEnvId<'db> {
    trait_env_from_module_resolution(db, module, module_resolution)
}

fn infer_function<'db>(
    db: &'db TestDb,
    module: Module<'db>,
    name: &str,
) -> (FuncBody<'db>, InferenceResult<'db>) {
    let info = function_infos(db, module)
        .into_iter()
        .find(|info| function_name(db, info.function) == name)
        .expect("function");
    let function = info.function;
    let body = function.body(db).expect("body");
    let module_resolution = hir_nameres::resolve_module(db, module);
    let lowered = TypeLowering::from_item_resolutions(
        db,
        &module_resolution.item_resolutions,
        BinderEnv::from_type_vars(&info.type_vars),
    )
    .lower_function(function);
    let body_map = body_map(db, &module_resolution, body);
    let ctx = BodyTyContext::new(
        module,
        body_map,
        info.type_vars,
        lowered.params,
        Some(lowered.ret),
    )
    .with_param_names(param_names(db, function.sig(db).params.atom()));
    (body, infer_body(db, body, ctx))
}

fn infer_module_function<'db>(
    db: &'db TestDb,
    module_id: ModuleId<'db>,
    name: &str,
) -> (FuncBody<'db>, InferenceResult<'db>) {
    infer_module_function_impl(db, module_id, name, false)
}

fn infer_module_function_with_solver<'db>(
    db: &'db TestDb,
    module_id: ModuleId<'db>,
    name: &str,
) -> (FuncBody<'db>, InferenceResult<'db>) {
    infer_module_function_impl(db, module_id, name, true)
}

fn infer_module_function_impl<'db>(
    db: &'db TestDb,
    module_id: ModuleId<'db>,
    name: &str,
    solve_obligations: bool,
) -> (FuncBody<'db>, InferenceResult<'db>) {
    let module = module_hir(db, module_id).expect("module hir");
    let info = function_infos(db, module)
        .into_iter()
        .find(|info| function_name(db, info.function) == name)
        .expect("function");
    let function = info.function;
    let body = function.body(db).expect("body");
    let env = nameres::module_env_for_hir_module(db, module_id, module);
    let scope = env.item_scope.clone().expect("item scope");
    let module_resolution = hir_nameres::resolve_module_with_imports(db, module, scope, &env);
    let lowerer = TypeLowering::from_item_resolutions(
        db,
        &module_resolution.item_resolutions,
        BinderEnv::from_type_vars(&info.type_vars),
    );
    let mut lowered = lowerer.lower_function(function);
    let mut normalizer = AliasNormalizer::new(db, module, &module_resolution.item_resolutions);
    lowered.scheme = normalizer.normalize_scheme(lowered.scheme);
    lowered.params = lowered
        .params
        .into_iter()
        .map(|param| normalizer.normalize_ty(param))
        .collect();
    lowered.ret = normalizer.normalize_ty(lowered.ret);
    let lookup = find_function_info(db, module, function.def_id_value(db)).expect("lookup");
    let body_map = body_resolution_for_function_with_imports(db, module, &lookup, Some(&env))
        .expect("body map");
    let mut ctx = BodyTyContext::new(
        module,
        body_map,
        info.type_vars,
        lowered.params,
        Some(lowered.ret),
    )
    .with_param_names(param_names(db, function.sig(db).params.atom()))
    .with_entry_module(module_id);
    if solve_obligations {
        let base_trait_env = crate::solver::trait_env_from_module_resolution_and_imports(
            db,
            module,
            &module_resolution,
            &env.import_surface(),
        );
        let trait_env = trait_env_with_givens(
            db,
            base_trait_env,
            lowered.scheme.body(db).preds(db).clone(),
        );
        ctx = ctx.with_trait_env(trait_env);
    }
    (body, infer_body(db, body, ctx))
}

fn infer_all_functions_with_solver<'db>(
    db: &'db TestDb,
    module: Module<'db>,
) -> Vec<(String, InferenceResult<'db>)> {
    let module_resolution = hir_nameres::resolve_module(db, module);
    let base_trait_env = trait_env(db, module, &module_resolution);
    function_infos(db, module)
        .into_iter()
        .filter_map(|info| {
            let body = info.function.body(db)?;
            let lowered = TypeLowering::from_item_resolutions(
                db,
                &module_resolution.item_resolutions,
                BinderEnv::from_type_vars(&info.type_vars),
            )
            .lower_function(info.function);
            let body_map = body_map(db, &module_resolution, body);
            let trait_env = trait_env_with_givens(
                db,
                base_trait_env,
                lowered.scheme.body(db).preds(db).clone(),
            );
            let ctx = BodyTyContext::new(
                module,
                body_map,
                info.type_vars,
                lowered.params,
                Some(lowered.ret),
            )
            .with_param_names(param_names(db, info.function.sig(db).params.atom()))
            .with_trait_env(trait_env);
            Some((
                function_name(db, info.function).to_owned(),
                infer_body(db, body, ctx),
            ))
        })
        .collect()
}

fn class_id<'db>(db: &'db TestDb, module: Module<'db>, name: &str) -> ClassId<'db> {
    for item in module.items(db) {
        if let Item::ClassDef(class) = item
            && class.def_id_value(db).name(db).as_deref() == Some(name)
        {
            return ClassId::User(class.def_id_value(db));
        }
    }
    panic!("class {name}");
}

fn adt_def<'db>(db: &'db TestDb, module: Module<'db>, name: &str) -> DefId<'db> {
    for item in module.items(db) {
        if let Item::AdtDef(adt) = item
            && adt.def_id_value(db).name(db).as_deref() == Some(name)
        {
            return adt.def_id_value(db);
        }
    }
    panic!("adt {name}");
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

fn solve_class_goal<'db>(
    db: &'db TestDb,
    env: TraitEnvId<'db>,
    class: ClassId<'db>,
    main: Ty<'db>,
    args: Vec<Ty<'db>>,
) -> Solution<'db> {
    let goal = Pred::in_class(db, class, main, args);
    solve(db, env, canonical_goal(db, goal))
}

fn solve_class_report<'db>(
    db: &'db TestDb,
    env: TraitEnvId<'db>,
    class: ClassId<'db>,
    main: Ty<'db>,
    args: Vec<Ty<'db>>,
) -> crate::SolverReport<'db> {
    let goal = Pred::in_class(db, class, main, args);
    solve_report(db, env, canonical_goal(db, goal))
}

fn return_expr<'db>(db: &'db TestDb, body: FuncBody<'db>) -> Id<Expr<'db>> {
    let stmt = body.stmts(db).get(body.top_level_stmts(db)[0]);
    match &stmt.kind {
        StmtKind::Return(Some(expr)) => *expr,
        _ => panic!("expected return expression"),
    }
}

#[test]
fn obligation_canonicalization_keeps_rigid_and_goal_variables_disjoint() {
    let db = TestDb::default();
    let mut table = InferTable::new(&db);
    let open = table.fresh_var();
    let mut canonicalizer = ObligationCanonicalizer::new(&db, &mut table, 1);

    let rigid = canonicalizer.ty(InferTy::BoundVar(0));
    let goal = canonicalizer.ty(open);

    assert!(matches!(rigid.kind(&db), TyKind::BoundVar(var) if var.index == 0));
    assert!(matches!(goal.kind(&db), TyKind::BoundVar(var) if var.index == 1));
    assert_eq!(canonicalizer.allowed_vars(), vec![1]);
}

#[test]
fn deferred_dependency_snapshots_follow_union_roots() {
    let db = TestDb::default();
    let mut engine = InferTable::new(&db);
    let first = engine.fresh_vid();
    let second = engine.fresh_vid();
    let unrelated = engine.fresh_vid();
    let first_before = engine.resolve(InferTy::Var(first));
    let second_before = engine.resolve(InferTy::Var(second));
    let unrelated_before = engine.resolve(InferTy::Var(unrelated));

    engine
        .unify(InferTy::Var(first), InferTy::Var(second))
        .unwrap();
    let InferTy::Var(current_root) = engine.resolve(InferTy::Var(first)) else {
        panic!("union of two open inference variables must remain open");
    };
    let stale_root = if current_root == first { second } else { first };
    let stale_before = if stale_root == first {
        first_before.clone()
    } else {
        second_before.clone()
    };
    let current_before = if current_root == first {
        first_before
    } else {
        second_before
    };
    assert_ne!(stale_root, current_root);

    // Do not inspect the snapshots between the union and the later binding.
    // A dirty-root set containing only `current_root` would miss the snapshot
    // keyed by `stale_root`, even though resolving that old handle follows the
    // union and observes the concrete value.
    engine
        .unify(
            InferTy::Var(current_root),
            InferTy::Named {
                ctor: TyCtor::Builtin(BuiltinTyCtor::Word),
                args: Vec::new(),
            },
        )
        .unwrap();
    let deferred = FxHashMap::from_iter([
        (0, FxHashMap::from_iter([(stale_root, stale_before)])),
        (1, FxHashMap::from_iter([(current_root, current_before)])),
        (2, FxHashMap::from_iter([(unrelated, unrelated_before)])),
    ]);

    assert_eq!(
        deferred_obligations_affected_by(&mut engine, &deferred),
        vec![0, 1]
    );
}

fn function_info_named<'db>(db: &'db TestDb, module: Module<'db>, name: &str) -> FunctionInfo<'db> {
    function_infos(db, module)
        .into_iter()
        .find(|info| function_name(db, info.function) == name)
        .expect("function")
}

fn assert_no_typeck(result: &InferenceResult<'_>) {
    assert!(
        result.diagnostics.is_empty(),
        "unexpected type diagnostics: {:?}",
        result.diagnostics
    );
}

fn has_user_obligation<'db>(
    db: &'db TestDb,
    result: &InferenceResult<'db>,
    class_name: &str,
    main: Ty<'db>,
    args: &[Ty<'db>],
) -> bool {
    result.obligations.iter().any(|obligation| {
        matches!(
            obligation.pred.kind(db),
            PredKind::InClass {
                class: ClassId::User(class),
                main: obligation_main,
                args: obligation_args,
            } if class.name(db).as_deref() == Some(class_name)
                && *obligation_main == main
                && obligation_args.as_slice() == args
        )
    })
}

#[test]
fn unannotated_function_scheme_uses_inferred_polymorphic_body_type() {
    let db = TestDb::default();
    let module = parse_module(&db, "function id(x) { return x; }");
    let info = function_info_named(&db, module, "id");
    let scheme = function_scheme_in_hir_module(&db, module, info.function.def_id_value(&db))
        .expect("scheme");

    assert_eq!(scheme.binder_count(&db), 1);
    let TyKind::Function { params, ret } = scheme.body(&db).ty(&db).kind(&db) else {
        panic!("expected function scheme");
    };
    assert_eq!(params.len(), 1);
    assert!(matches!(
        params[0].kind(&db),
        TyKind::BoundVar(var) if var.index == 0
    ));
    assert!(matches!(
        ret.kind(&db),
        TyKind::BoundVar(var) if var.index == 0
    ));
}

#[test]
fn contract_entry_dispatch_uses_inferred_return_type() {
    let mut db = TestDb::default();
    let key = insert_module_source(
        &mut db,
        &["main"],
        r#"
contract Answer {
  public function main() {
return 42;
  }
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let hir_module = module_hir(&db, module).expect("module hir");
    let contract = hir_module
        .items(&db)
        .iter()
        .find_map(|item| match item {
            Item::ContractDef(contract) => Some(*contract),
            _ => None,
        })
        .expect("contract");
    let surface = crate::contract_dispatch_surface(&db, hir_module, contract);

    assert_eq!(surface.methods.len(), 1);
    assert_eq!(surface.methods[0].outputs.len(), 1);
    assert_eq!(surface.methods[0].outputs[0].ty.to_string(), "uint256");
}

#[test]
fn inference_result_records_comptime_obligation_sites() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
function need(comptime x: word) -> comptime word {
  return x;
}

function g() -> comptime word {
  let y : comptime word = need(2);
  return y;
}

function f(x: word) -> comptime word {
  match x {
  | comptime 1 => return need(2);
  | _ => return 0;
  }
}
"#,
    );
    let (_, g_result) = infer_function(&db, module, "g");

    assert!(
        g_result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(obligation.kind, ComptimeObligationKind::LetInit { .. })),
        "{:?}",
        g_result.comptime_obligations
    );
    assert!(
        g_result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(obligation.kind, ComptimeObligationKind::CallParam { .. })),
        "{:?}",
        g_result.comptime_obligations
    );
    assert!(
        g_result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(obligation.kind, ComptimeObligationKind::Return { .. })),
        "{:?}",
        g_result.comptime_obligations
    );

    let (_, f_result) = infer_function(&db, module, "f");
    assert!(
        f_result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(
                obligation.kind,
                ComptimeObligationKind::PatternLabel { .. }
            )),
        "{:?}",
        f_result.comptime_obligations
    );
}

#[test]
fn inferred_integer_let_records_comptime_obligation() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
function f() -> word {
  let x = wordToInteger(20);
  return wordFromInteger(x);
}
"#,
    );
    let (_, result) = infer_function(&db, module, "f");

    assert!(
        result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(
                &obligation.kind,
                ComptimeObligationKind::LetInit { name, .. } if name == "x"
            )),
        "{:?}",
        result.comptime_obligations
    );
}

#[test]
fn comptime_only_types_cover_params_returns_typed_lets_and_call_args() {
    let mut db = TestDb::default();
    let std_path = PathBuf::from("/std/std.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let std_file = source_file_at_path(
        &db,
        &std_path,
        r#"
export { string };
data string;
"#,
    );
    let main_file = source_file_at_path(
        &db,
        &main_path,
        r#"
import std.{string};

type Text = string;
type Big = integer;

function explicitlyNeedsText(comptime value: Text) -> () {
  return ();
}

function explicitlyNeedsBig(comptime value: Big) -> () {
  return ();
}

function textParamIsComptime(value: Text) -> () {
  return explicitlyNeedsText(value);
}

function bigParamIsComptime(value: Big) -> () {
  return explicitlyNeedsBig(value);
}

function takesText(value: Text) -> () {
  return ();
}

function takesBig(value: Big) -> () {
  return ();
}

function exerciseText(value: Text) -> Text {
  let copy: Text = value;
  takesText(copy);
  return copy;
}

function exerciseBig(value: Big) -> Big {
  let copy: Big = value;
  takesBig(copy);
  return copy;
}
"#,
    );
    let std_key = module_key_for_path(LibraryId::Std, &PathBuf::from("/std"), &std_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(std_key, std_file);
    db.insert_module_file(main_key.clone(), main_file);
    let main_module = module_id_from_key(&db, &main_key);

    for name in ["textParamIsComptime", "bigParamIsComptime"] {
        let (_, result) = infer_module_function(&db, main_module, name);
        assert_no_typeck(&result);
    }

    for name in ["exerciseText", "exerciseBig"] {
        let (_, result) = infer_module_function(&db, main_module, name);
        assert_no_typeck(&result);
        assert!(
            result
                .comptime_obligations
                .iter()
                .any(|obligation| matches!(
                    obligation.kind,
                    ComptimeObligationKind::LetInit { .. }
                )),
            "{name}: {:?}",
            result.comptime_obligations
        );
        assert!(
            result
                .comptime_obligations
                .iter()
                .any(|obligation| matches!(
                    obligation.kind,
                    ComptimeObligationKind::CallParam { .. }
                )),
            "{name}: {:?}",
            result.comptime_obligations
        );
        assert!(
            result
                .comptime_obligations
                .iter()
                .any(|obligation| matches!(obligation.kind, ComptimeObligationKind::Return { .. })),
            "{name}: {:?}",
            result.comptime_obligations
        );
    }

    let diagnostics = module_typeck_diagnostics(&db, main_module)
        .iter()
        .map(|diagnostic| diagnostic.lower(&db))
        .collect::<Vec<_>>();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn string_literals_and_concat_lit_use_str_conversion_only_at_literal_sites() {
    let mut db = TestDb::default();
    let std_path = PathBuf::from("/std/std.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let std_file = source_file_at_path(
        &db,
        &std_path,
        r#"
export { memory(*), string, concatLit, strlenLit };
data memory(a) = memory(word);
data string;
function concatLit(comptime lhs: string, comptime rhs: string) -> string { return lhs; }
function strlenLit(comptime value: string) -> word { return 0; }
"#,
    );
    let main_file = source_file_at_path(
        &db,
        &main_path,
        r#"
import std;
import std.{memory, string, strlenLit};

data Tag = Tag(word);
instance Tag : Str {
  function fromString(comptime value: string) -> Tag {
    return Tag(strlenLit(value));
  }
}

function literal() -> memory(string) { return "hello"; }
function concatLit(lhs: word, rhs: word) -> word { return lhs; }
function concatenated() -> memory(string) { return std.concatLit("he", "llo"); }
function explicit(value: string) -> memory(string) { return Str.fromString(value); }
function tagged() -> Tag { return "abcd"; }
function taggedFromLet() -> Tag {
  let value = "abcd";
  return Str.fromString(value);
}
function inferredLiteral() -> () { let value = "x"; return (); }
function inferredConcat() -> () { let value = std.concatLit("a", "b"); return (); }
function consumePair(value: (string, word)) -> word { return 0; }
function inferredTuple() -> word {
  let value = ("x", 0);
  return consumePair(value);
}
function inferredComptimeParam() -> () {
  let sink = lam (comptime value) -> () { return (); };
  sink("x");
  return ();
}
function invalidConcat() -> memory(string) { return concatLit(1, 2); }
function makeWord() -> word { return 0; }
function invalidSource() -> memory(string) {
  let value;
  let result: memory(string) = Str.fromString(value);
  value = makeWord();
  return result;
}
function runtime(value: string) -> memory(string) { return value; }
"#,
    );
    let std_key = module_key_for_path(LibraryId::Std, &PathBuf::from("/std"), &std_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(std_key, std_file);
    db.insert_module_file(main_key.clone(), main_file);
    assert!(
        parse_diagnostics(&db, main_file).is_empty(),
        "{:?}",
        parse_diagnostics(&db, main_file)
    );
    let main_module = module_id_from_key(&db, &main_key);
    let module_diagnostics = module_typeck_diagnostics(&db, main_module)
        .iter()
        .map(|diagnostic| diagnostic.lower(&db))
        .collect::<Vec<_>>();
    assert_eq!(module_diagnostics.len(), 3, "{module_diagnostics:?}");
    assert!(
        module_diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_deref() == Some("SC0201")
                && diagnostic.message.contains("memory")
                && diagnostic.message.contains("string")
        }),
        "{module_diagnostics:?}"
    );

    assert!(
        module_diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_deref() == Some("SC0201")
                && diagnostic.message.contains("string")
                && diagnostic.message.contains("word")
        }),
        "{module_diagnostics:?}"
    );

    for name in [
        "literal",
        "concatenated",
        "explicit",
        "tagged",
        "taggedFromLet",
    ] {
        let (_, result) = infer_module_function(&db, main_module, name);
        assert_no_typeck(&result);
        assert!(
            result.obligations.iter().any(|obligation| {
                matches!(
                    obligation.pred.kind(&db),
                    PredKind::InClass {
                        class: ClassId::Builtin(BuiltinClassId::Str),
                        ..
                    }
                )
            }),
            "{name}: {:?}",
            result.obligations
        );
    }

    for name in [
        "inferredLiteral",
        "inferredConcat",
        "inferredTuple",
        "inferredComptimeParam",
    ] {
        let (_, result) = infer_module_function(&db, main_module, name);
        assert_no_typeck(&result);
    }
}

#[test]
fn array_literals_infer_canonical_memory_dyn_array_and_empty_uses_context() {
    let (db, key) = db_with_array_std(
        r#"
import std.{memory, DynArray};

function filled(x:word, y:word) -> memory(DynArray(word)) { return [x, y]; }
function empty() -> memory(DynArray(word)) { return []; }
"#,
    );
    let module = module_id_from_key(&db, &key);
    let word = Ty::word(&db);
    let dyn_array = canonical_std_adt_ty(&db, "DynArray", vec![word]);
    let expected = canonical_std_adt_ty(&db, "memory", vec![dyn_array]);

    for name in ["filled", "empty"] {
        let (body, result) = infer_module_function(&db, module, name);
        assert_no_typeck(&result);
        let expr = return_expr(&db, body);
        assert!(matches!(body.exprs(&db).get(expr).kind, ExprKind::Array(_)));
        assert_eq!(result.expr_ty(body, expr), Some(expected), "{name}");
    }
}

#[test]
fn array_literal_rejects_mixed_element_types() {
    let (db, key) = db_with_array_std(
        r#"
import std.{memory, DynArray};

function mixed(x:word, flag:bool) -> memory(DynArray(word)) {
  return [x, flag];
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let (_, result) = infer_module_function(&db, module, "mixed");

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, TypeckDiagnostic::Mismatch { .. })),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn array_string_literals_use_memory_string_from_memory_and_storage_contexts() {
    let (db, key) = db_with_array_std(
        r#"
import std;
import std.{memory, storage, DynArray, array, string, Typedef, CanStore};

function inMemory() -> memory(DynArray(memory(string))) {
  return ["hello"];
}

function explicitConversion() -> memory(DynArray(memory(string))) {
  return [Str.fromString("hello")];
}

function concatenated() -> memory(DynArray(memory(string))) {
  return [std.concatLit("hel", "lo")];
}

function conditional(flag:bool) -> memory(DynArray(memory(string))) {
  return [if (flag) then "yes" else "no"];
}

contract C {
  names:array(string);

  function setNames() -> () {
    names = ["alice", "bob"];
    return ();
  }

  function clearNames() -> () {
    names = [];
    return ();
  }
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let string = canonical_std_adt_ty(&db, "string", Vec::new());
    let memory_string = canonical_std_adt_ty(&db, "memory", vec![string]);
    let dyn_array = canonical_std_adt_ty(&db, "DynArray", vec![memory_string]);
    let memory_array = canonical_std_adt_ty(&db, "memory", vec![dyn_array]);

    for name in [
        "inMemory",
        "explicitConversion",
        "concatenated",
        "conditional",
    ] {
        let (body, result) = infer_module_function_with_solver(&db, module, name);
        assert_no_typeck(&result);
        let array = return_expr(&db, body);
        assert_eq!(result.expr_ty(body, array), Some(memory_array), "{name}");
        let ExprKind::Array(elems) = &body.exprs(&db).get(array).kind else {
            panic!("array literal");
        };
        let actual_elem = result.expr_ty(body, elems[0]).expect("array element type");
        assert_eq!(
            actual_elem,
            memory_string,
            "{name}: {}",
            Pred::in_class(
                &db,
                ClassId::Builtin(BuiltinClassId::Str),
                actual_elem,
                Vec::new(),
            )
            .display(&db)
        );
    }

    let storage_string = canonical_std_adt_ty(&db, "storage", vec![string]);
    let word = Ty::word(&db);
    for name in ["setNames", "clearNames"] {
        let (body, result) = infer_module_function_with_solver(&db, module, name);
        assert_no_typeck(&result);
        let rhs = body
            .top_level_stmts(&db)
            .iter()
            .find_map(|stmt| match body.stmts(&db).get(*stmt).kind {
                StmtKind::Assign { rhs, .. } => Some(rhs),
                _ => None,
            })
            .expect("array field assignment");
        let actual_rhs = result.expr_ty(body, rhs).expect("array literal type");
        assert_eq!(
            actual_rhs,
            memory_array,
            "{name}: actual={}, obligations={:?}",
            Pred::in_class(
                &db,
                ClassId::Builtin(BuiltinClassId::Str),
                actual_rhs,
                Vec::new(),
            )
            .display(&db),
            result
                .obligations
                .iter()
                .map(|obligation| obligation.pred.display(&db))
                .collect::<Vec<_>>()
        );
        assert!(
            has_user_obligation(&db, &result, "CanStore", storage_string, &[memory_string],),
            "{name}: {:?}",
            result.obligations
        );
        assert!(
            has_user_obligation(&db, &result, "Typedef", memory_string, &[word]),
            "{name}: {:?}",
            result.obligations
        );
    }
}

#[test]
fn memory_dyn_array_index_returns_element_and_requires_word_typedefs() {
    let (db, key) = db_with_array_std(
        r#"
import std.{memory, DynArray, uint256, Typedef};

function read(xs:memory(DynArray(word)), i:uint256) -> word {
  return xs[i];
}

function write(xs:memory(DynArray(word)), i:uint256, value:word) -> () {
  xs[i] = value;
  return ();
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let (body, result) = infer_module_function(&db, module, "read");
    assert_no_typeck(&result);
    let indexed = return_expr(&db, body);
    assert!(matches!(
        body.exprs(&db).get(indexed).kind,
        ExprKind::Index { .. }
    ));
    assert_eq!(result.expr_ty(body, indexed), Some(Ty::word(&db)));

    let uint256 = canonical_std_adt_ty(&db, "uint256", Vec::new());
    let word = Ty::word(&db);
    assert!(
        has_user_obligation(&db, &result, "Typedef", uint256, &[word]),
        "{:?}",
        result.obligations
    );
    assert!(
        has_user_obligation(&db, &result, "Typedef", word, &[word]),
        "{:?}",
        result.obligations
    );

    let (_, write) = infer_module_function(&db, module, "write");
    assert!(
        write.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            TypeckDiagnostic::Mismatch { expected, .. }
                if expected == "assignable storage-backed index"
        )),
        "{:?}",
        write.diagnostics
    );
}

#[test]
fn direct_storage_array_handle_assignment_is_a_raw_rebind() {
    let (db, key) = db_with_array_std(
        r#"
import std.{storage, array, string, CanStore};

function rebind(
  lhs:storage(array(string)),
  rhs:storage(array(string))
) -> () {
  lhs = rhs;
  return ();
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let (_, result) = infer_module_function_with_solver(&db, module, "rebind");
    assert_no_typeck(&result);

    let string = canonical_std_adt_ty(&db, "string", Vec::new());
    let array = canonical_std_adt_ty(&db, "array", vec![string]);
    let storage_array = canonical_std_adt_ty(&db, "storage", vec![array]);
    assert!(
        !has_user_obligation(&db, &result, "CanStore", storage_array, &[storage_array],),
        "{:?}",
        result.obligations
    );
}

#[test]
fn storage_load_and_assign_use_can_store_result_improvement() {
    let (db, key) = db_with_array_std(
        r#"
import std.{memory, storage, CanStore};

data Blob;

instance storage(Blob):CanStore(memory(Blob)) {
  function store(dst:storage(Blob), value:memory(Blob)) -> () { return (); }
  function load(dst:storage(Blob)) -> memory(Blob) { return memory(0); }
}

contract C {
  value:Blob;

  function write(src:memory(Blob)) -> () {
    value = src;
    return ();
  }

  function read() -> memory(Blob) {
    return value;
  }
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    for name in ["write", "read"] {
        let (_, result) = infer_module_function_with_solver(&db, module, name);
        assert_no_typeck(&result);
    }
}

#[test]
fn compound_storage_array_index_recognizes_parameter_handles() {
    let (db, key) = db_with_array_std(
        r#"
import std.{*};

function bump(
  values:storage(array(uint256)),
  index:uint256,
  delta:uint256
) -> () {
  values[index] += delta;
  return ();
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let (_, result) = infer_module_function_with_solver(&db, module, "bump");
    assert_no_typeck(&result);

    let uint256 = canonical_std_adt_ty(&db, "uint256", Vec::new());
    let storage_uint256 = canonical_std_adt_ty(&db, "storage", vec![uint256]);
    assert!(
        has_user_obligation(&db, &result, "CanStore", storage_uint256, &[uint256],),
        "{:?}",
        result.obligations
    );
}

#[test]
fn storage_index_numeric_guard_rejects_shadowed_uint_names() {
    let (db, key) = db_with_array_std(
        r#"
data uint;
data uint256;
"#,
    );
    let module_id = module_id_from_key(&db, &key);
    let module = module_hir(&db, module_id).expect("module hir");

    assert!(super::expr::is_storage_index_word_numeric_shape(
        &db,
        TyCtor::Builtin(BuiltinTyCtor::Word),
        0,
    ));
    let canonical_uint256 =
        crate::support::canonical_std_adt_def(&db, "uint256").expect("std uint256");
    assert!(super::expr::is_storage_index_word_numeric_shape(
        &db,
        TyCtor::User(UserTyCtor {
            def: canonical_uint256,
            kind: UserTyCtorKind::Adt,
        }),
        0,
    ));

    for name in ["uint", "uint256"] {
        let shadow = adt_def(&db, module, name);
        assert!(
            !super::expr::is_storage_index_word_numeric_shape(
                &db,
                TyCtor::User(UserTyCtor {
                    def: shadow,
                    kind: UserTyCtorKind::Adt,
                }),
                0,
            ),
            "shadowed {name} must not be treated as a storage index numeric type"
        );
    }
}

#[test]
fn storage_ref_annotations_are_checked_without_widening_array_literal_routing() {
    let (db, key) = db_with_array_std(
        r#"
import std.{*};

contract C {
  xs:array(uint256);

  function good(i:uint256, value:uint256) -> () {
    (xs : storage(array(uint256)))[i] = value;
    return ();
  }

  function bad(i:uint256) -> () {
    (xs : storage(array(address)))[i] = uint256(1);
    return ();
  }

  function annotatedLiteral() -> () {
    xs = ([uint256(1)] : memory(DynArray(uint256)));
    return ();
  }
}
"#,
    );
    let module_id = module_id_from_key(&db, &key);

    let (body, good) = infer_module_function_with_solver(&db, module_id, "good");
    assert_no_typeck(&good);
    let annotation = body
        .exprs(&db)
        .iter()
        .find_map(|(id, expr)| matches!(expr.kind, ExprKind::TypeAnnot { .. }).then_some(id))
        .expect("storage array annotation");
    let uint256 = canonical_std_adt_ty(&db, "uint256", Vec::new());
    let array = canonical_std_adt_ty(&db, "array", vec![uint256]);
    let storage_array = canonical_std_adt_ty(&db, "storage", vec![array]);
    assert_eq!(good.expr_ty(body, annotation), Some(storage_array));

    let (body, bad) = infer_module_function_with_solver(&db, module_id, "bad");
    assert!(
        bad.diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, TypeckDiagnostic::Mismatch { .. })),
        "{:?}",
        bad.diagnostics
    );
    let annotation = body
        .exprs(&db)
        .iter()
        .find_map(|(id, expr)| matches!(expr.kind, ExprKind::TypeAnnot { .. }).then_some(id))
        .expect("mismatched storage array annotation");
    assert_eq!(bad.expr_ty(body, annotation), Some(Ty::error(&db)));

    let (_, annotated_literal) =
        infer_module_function_with_solver(&db, module_id, "annotatedLiteral");
    assert!(
        annotated_literal
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, TypeckDiagnostic::Mismatch { .. })),
        "{:?}",
        annotated_literal.diagnostics
    );

    let module = module_hir(&db, module_id).expect("module hir");
    let plan = crate::frontend_desugar_plan(&db, module);
    let annotated_literal = plan
        .bodies
        .iter()
        .find(|body| body.function_name == "annotatedLiteral")
        .expect("annotatedLiteral desugar plan");
    assert!(
        annotated_literal
            .transforms
            .iter()
            .any(|transform| matches!(
                transform,
                crate::FrontendTransform::FieldWrite { hook, .. }
                    if hook.starts_with("Assign.assign(")
            )),
        "{:?}",
        annotated_literal.transforms
    );
}

#[test]
fn storage_array_index_alias_and_literal_assignment_preserve_reference_types() {
    let (db, key) = db_with_array_std(
        r#"
import std.{storage, array, uint256, Typedef, CanStore};

contract C {
  xs:array(word);
  n:word;

  function read(i:uint256) -> word { return xs[i]; }

  function aliasRead(i:uint256) -> word {
    let ys = xs;
    return ys[i];
  }

  function aliasWrite(i:uint256, value:word) -> () {
    let ys = xs;
    ys[i] = value;
    return ();
  }

  function set(x:word) -> () {
    xs = [x, x];
    return ();
  }

  function bad(x:word) -> () {
    n = [x];
    return ();
  }
}
"#,
    );
    let module = module_id_from_key(&db, &key);

    for name in ["read", "aliasRead"] {
        let (body, result) = infer_module_function(&db, module, name);
        assert_no_typeck(&result);
        let indexed = body
            .exprs(&db)
            .iter()
            .find_map(|(id, expr)| matches!(expr.kind, ExprKind::Index { .. }).then_some(id))
            .expect("indexed expression");
        assert_eq!(result.expr_ty(body, indexed), Some(Ty::word(&db)), "{name}");
    }

    let (_, alias_write) = infer_module_function_with_solver(&db, module, "aliasWrite");
    assert_no_typeck(&alias_write);
    let storage_word = canonical_std_adt_ty(&db, "storage", vec![Ty::word(&db)]);
    let word = Ty::word(&db);
    assert!(
        has_user_obligation(&db, &alias_write, "CanStore", storage_word, &[word],),
        "{:?}",
        alias_write.obligations
    );

    let (body, result) = infer_module_function(&db, module, "set");
    assert_no_typeck(&result);
    let lhs = body
        .top_level_stmts(&db)
        .iter()
        .find_map(|stmt| match body.stmts(&db).get(*stmt).kind {
            StmtKind::Assign { lhs, .. } => Some(lhs),
            _ => None,
        })
        .expect("field assignment");
    let array = canonical_std_adt_ty(&db, "array", vec![Ty::word(&db)]);
    let storage_array = canonical_std_adt_ty(&db, "storage", vec![array]);
    assert_eq!(result.expr_ty(body, lhs), Some(storage_array));
    let storage_word = canonical_std_adt_ty(&db, "storage", vec![Ty::word(&db)]);
    let word = Ty::word(&db);
    assert!(
        has_user_obligation(&db, &result, "CanStore", storage_word, &[word]),
        "{:?}",
        result.obligations
    );
    assert!(
        has_user_obligation(&db, &result, "Typedef", word, &[word]),
        "{:?}",
        result.obligations
    );

    let (_, bad) = infer_module_function(&db, module, "bad");
    assert!(
        bad.diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, TypeckDiagnostic::Mismatch { .. })),
        "{:?}",
        bad.diagnostics
    );
}

#[test]
fn array_literal_contract_field_write_plan_uses_store_array_lit() {
    let (db, key) = db_with_array_std(
        r#"
import std.{storage, array};

contract C {
  xs:array(word);

  function set(x:word) -> () {
    xs = [x];
    return ();
  }
}
"#,
    );
    let module_id = module_id_from_key(&db, &key);
    let module = module_hir(&db, module_id).expect("module hir");
    let plan = crate::frontend_desugar_plan(&db, module);
    let set = plan
        .bodies
        .iter()
        .find(|body| body.function_name == "set")
        .expect("set desugar plan");
    assert!(
        set.transforms.iter().any(|transform| matches!(
            transform,
            crate::FrontendTransform::FieldWrite { hook, .. }
                if hook.starts_with("storeArrayLit(")
        )),
        "{:?}",
        set.transforms
    );
}

#[test]
fn module_local_string_and_integer_adts_remain_runtime_types() {
    let diagnostics = lowered_module_typeck_diagnostics(
        r#"
data string = RuntimeString(word);
data integer = RuntimeInteger(word);

function takesString(value: string) -> () {
  return ();
}

function takesInteger(value: integer) -> () {
  return ();
}

function exercise(value: word) -> () {
  takesString(string.RuntimeString(value));
  takesInteger(integer.RuntimeInteger(value));
  return ();
}
"#,
    );

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn module_local_string_adt_rejects_primitive_string_patterns() {
    let diagnostics = lowered_module_typeck_diagnostics(
        r#"
data string = RuntimeString(word);

function inspect(value: string) -> word {
  match value {
  | "a" => return 1;
  | _ => return 0;
  }
}
"#,
    );

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some("SC0201")),
        "{diagnostics:?}"
    );
}

#[test]
fn contract_local_string_and_integer_adts_remain_runtime_types() {
    let diagnostics = lowered_module_typeck_diagnostics(
        r#"
contract RuntimeNames {
  data string = RuntimeString(word);
  data integer = RuntimeInteger(word);

  function takesString(value: string) -> () {
    return ();
  }

  function takesInteger(value: integer) -> () {
    return ();
  }

  function exercise(value: word) -> () {
    takesString(string.RuntimeString(value));
    takesInteger(integer.RuntimeInteger(value));
    return ();
  }
}
"#,
    );

    // The bare test module has no standard contract-default classes; those
    // unrelated resolution errors must not hide an identity-based SC0240.
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.as_deref() != Some("SC0240")),
        "{diagnostics:?}"
    );
}

#[test]
fn inferred_string_let_records_comptime_obligation() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
function f() -> word {
  let message = "hello";
  return 0;
}
"#,
    );
    let (_, result) = infer_function(&db, module, "f");

    assert!(
        result
            .comptime_obligations
            .iter()
            .any(|obligation| matches!(
                &obligation.kind,
                ComptimeObligationKind::LetInit { name, .. } if name == "message"
            )),
        "{:?}",
        result.comptime_obligations
    );
}

#[test]
fn unify_occurs_check_rejects_recursive_type() {
    let db = TestDb::default();
    let mut table = InferTable::new(&db);
    let var = table.fresh_vid();
    let recursive = InferTy::Function {
        params: vec![InferTy::Var(var)],
        ret: Box::new(table.from_ty(Ty::word(&db))),
    };

    let err = table
        .unify(InferTy::Var(var), recursive)
        .expect_err("occurs");
    assert!(matches!(err, UnifyError::Occurs { .. }));
}

#[test]
fn unify_trial_rolls_back_successful_snapshot() {
    let db = TestDb::default();
    let mut table = InferTable::new(&db);
    let var = table.fresh_vid();
    let word = table.from_ty(Ty::word(&db));

    assert!(table.can_unify(InferTy::Var(var), word.clone()));
    assert_eq!(table.ground_ty(InferTy::Var(var)), Ty::unknown(&db));

    table
        .unify(InferTy::Var(var), word)
        .expect("committed unify");
    assert_eq!(table.ground_ty(InferTy::Var(var)), Ty::word(&db));
}

#[test]
fn scheme_instantiation_reuses_one_fresh_var_per_binder() {
    let db = TestDb::default();
    let bound = Ty::bound(&db, 0);
    let scheme = TyScheme::new(
        &db,
        1,
        QualTy::monotype(&db, Ty::function(&db, vec![bound], bound)),
    );
    let mut table = InferTable::new(&db);
    let instantiated = table.instantiate_scheme(scheme);

    let InferTy::Function { params, ret } = instantiated.ty else {
        panic!("function scheme");
    };
    let InferTy::Var(param_var) = &params[0] else {
        panic!("fresh param var");
    };
    let InferTy::Var(ret_var) = &*ret else {
        panic!("fresh ret var");
    };
    assert_eq!(param_var, ret_var);
}

#[test]
fn ambiguous_integer_literal_defaults_to_word() {
    let db = TestDb::default();
    let module = parse_module(&db, "function f() -> word { return 1; }");
    let (body, result) = infer_function(&db, module, "f");
    assert!(result.diagnostics.is_empty());

    let expr = return_expr(&db, body);
    assert_eq!(result.expr_ty(body, expr), Some(Ty::word(&db)));
    assert_eq!(result.obligations.len(), 1);
    assert_eq!(result.obligations[0].pred.display(&db), "word:Int");
}

#[test]
fn end_to_end_body_infers_word_arithmetic() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
class t:Add {
  function add(l:t, r:t) -> t;
}

instance word:Add {
  function add(l:word, r:word) -> word {
return primAddWord(l, r);
  }
}

function f(x: word) -> word { return x + 1; }
"#,
    );
    let (body, result) = infer_function(&db, module, "f");
    assert!(result.diagnostics.is_empty());

    let expr = return_expr(&db, body);
    assert!(matches!(
        &body.exprs(&db).get(expr).kind,
        ExprKind::BinOp {
            op,
            ..
        } if *op.atom() == BinOp::Add
    ));
    assert_eq!(result.expr_ty(body, expr), Some(Ty::word(&db)));
    assert!(
        result
            .obligations
            .iter()
            .any(|obligation| obligation.pred.display(&db) == "word:Int"),
        "{:?}",
        result.obligations
    );
}

#[test]
fn class_method_call_emits_obligation() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a: Enum {
  function fromEnum(x : a) -> word;
}

data Food = Curry | Beans | Other;

function main() -> word {
  return Enum.fromEnum(Food.Beans);
}
"#,
    );
    let (_, result) = infer_function(&db, module, "main");
    assert_no_typeck(&result);
    assert!(
        result
            .obligations
            .iter()
            .any(|obligation| obligation.pred.display(&db).contains(":Enum")),
        "expected Enum obligation, got {:?}",
        result.obligations
    );
}

#[test]
fn pair_domains_preserve_source_call_arity_and_explicit_tuple_arguments() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
function call_zero(f : () -> word) -> word {
  return f();
}

function call_pair(f : (word, bool) -> word, x : word, y : bool) -> word {
  return f(x, y);
}

function call_tuple(f : ((word, bool)) -> word, x : (word, bool)) -> word {
  return f(x);
}
"#,
    );

    for (name, result) in infer_all_functions_with_solver(&db, module) {
        assert!(
            result.diagnostics.is_empty(),
            "{name}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn class_method_local_forall_is_lowered_as_a_method_binder() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall b.
class b:IsA {
  forall a.
  function ais(p : (a,b)) -> a;
}
"#,
    );
    let resolution = hir_nameres::resolve_module(&db, module);
    assert!(resolution.diagnostics.is_empty(), "{resolution:?}");
    let class = module
        .items(&db)
        .iter()
        .find_map(|item| match item {
            Item::ClassDef(class) => Some(*class),
            _ => None,
        })
        .expect("class");
    let method = &class.methods(&db)[0];
    let method_type_vars = class_method_type_vars(&db, class, method);
    let scheme = TypeLowering::from_item_resolutions(
        &db,
        &resolution.item_resolutions,
        BinderEnv::from_type_vars(&method_type_vars),
    )
    .lower_class_method(class, method);

    assert_eq!(scheme.binder_count(&db), 2);
    let TyKind::Function { params, ret } = scheme.body(&db).ty(&db).kind(&db) else {
        panic!("method should lower to a function");
    };
    assert_eq!(params.len(), 1);
    let TyKind::Named {
        ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
        args,
    } = params[0].kind(&db)
    else {
        panic!("method parameter should be a pair");
    };
    assert!(matches!(args[0].kind(&db), TyKind::BoundVar(var) if var.index == 1));
    assert!(matches!(args[1].kind(&db), TyKind::BoundVar(var) if var.index == 0));
    assert!(matches!(ret.kind(&db), TyKind::BoundVar(var) if var.index == 1));
}

#[test]
fn method_local_forall_survives_instance_signature_soundness() {
    let diagnostics = lowered_module_typeck_diagnostics(
        r#"
forall b.
class b:IsA {
  forall a.
  function ais(x : a, witness : b) -> a;
}

instance word:IsA {
  forall a.
  function ais(x : a, witness : word) -> a {
    return x;
  }
}
"#,
    );

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn builtin_str_instance_requires_from_string() {
    let (db, key) = db_with_main_typeck(
        r#"
data Wrapped = Wrapped(word);
instance Wrapped:Str {}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module);

    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            TypeckDiagnostic::IncompleteInstance { class, missing, .. }
                if class == "Str" && missing == &["fromString".to_owned()]
        )),
        "{diagnostics:?}"
    );
}

#[test]
fn builtin_str_instance_rejects_unknown_methods() {
    let (db, key) = db_with_main_typeck(
        r#"
data Wrapped = Wrapped(word);
instance Wrapped:Str {
  function unexpected(x:word) -> word { return x; }
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module);

    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            TypeckDiagnostic::UnknownInstanceMethod { name, .. }
                if name == "Str.unexpected"
        )),
        "{diagnostics:?}"
    );
}

#[test]
fn builtin_str_instance_rejects_wrong_from_string_signature() {
    let (db, key) = db_with_main_typeck(
        r#"
data Wrapped = Wrapped(word);
instance Wrapped:Str {
  function fromString(s:word) -> Wrapped { return Wrapped(s); }
}
"#,
    );
    let module = module_id_from_key(&db, &key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module);

    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            TypeckDiagnostic::InvalidInstanceMethodSignature { method, .. }
                if method == "fromString"
        )),
        "{diagnostics:?}"
    );
}

#[test]
fn builtin_str_ground_instance_rejects_overlapping_source_instance() {
    let mut db = TestDb::default();
    let std_path = PathBuf::from("/std/std.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let std_file = source_file_at_path(
        &db,
        &std_path,
        r#"
export { memory(*), string };
data memory(a) = memory(word);
data string;
"#,
    );
    let main_file = source_file_at_path(
        &db,
        &main_path,
        r#"
import std.{*};

instance string:Str {
  function fromString(comptime value:string) -> string { return value; }
}

instance memory(string):Str {
  function fromString(comptime value:string) -> memory(string) {
    return Str.fromString(value);
  }
}
"#,
    );
    let std_key = module_key_for_path(LibraryId::Std, &PathBuf::from("/std"), &std_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(std_key, std_file);
    db.insert_module_file(main_key.clone(), main_file);
    let module = module_id_from_key(&db, &main_key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module);

    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| matches!(
                diagnostic,
                TypeckDiagnostic::OverlappingInstance {
                    overlaps_span: None,
                    ..
                }
            ))
            .count(),
        2,
        "{diagnostics:?}"
    );
}

#[test]
fn comptime_numeric_scrutinees_accept_integer_literal_patterns() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
function classify_word(comptime x : word) -> word {
  match x {
  | 0 => return 10;
  | _ => return 20;
  }
}

function classify_integer(comptime x : integer) -> word {
  match x {
  | 0 => return 10;
  | _ => return 20;
  }
}
"#,
    );

    for (name, result) in infer_all_functions_with_solver(&db, module) {
        assert!(
            result.diagnostics.is_empty(),
            "{name}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn unconstrained_phantom_constructor_result_is_ambiguous() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Foo(a) = Foo(word);

forall a . function read(x : Foo(a)) -> word {
  return 0;
}

function main() -> word {
  return read(Foo(42));
}
"#,
    );
    let (_, result) = infer_function(&db, module, "main");

    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| matches!(diagnostic, TypeckDiagnostic::AmbiguousInferredType { .. })),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn payload_constrained_constructor_result_is_not_phantom() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Box(a) = Box(a);

forall a . function unwrap(x : Box(a)) -> a {
  match x {
  | Box(value) => return value;
  }
}

function main() -> word {
  return unwrap(Box(42));
}
"#,
    );
    let (_, result) = infer_function(&db, module, "main");

    assert_no_typeck(&result);
}

#[test]
fn expected_type_constrains_phantom_constructor_result() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Foo(a) = Foo(word);

function main() -> Foo(word) {
  return Foo(42);
}
"#,
    );
    let (_, result) = infer_function(&db, module, "main");

    assert_no_typeck(&result);
}

#[test]
fn storage_word_field_read_loads_as_word_without_context() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data storage(t) = storage(word);

forall a b.
class a:CanStore(b) {
  function store(r:a, v:b) -> ();
  function load(r:a) -> b;
}

instance storage(word):CanStore(word) {
  function store(dst: storage(word), src: word) -> () {
return ();
  }

  function load(src: storage(word)) -> word {
return 0;
  }
}

contract C {
  value: word;

  function get() {
let x = value;
return x;
  }
}
"#,
    );
    let (body, result) = infer_function(&db, module, "get");
    assert_no_typeck(&result);

    let value_expr = body
        .exprs(&db)
        .iter()
        .find_map(|(expr_id, expr)| match &expr.kind {
            ExprKind::Ident(name) if (*name.atom()).text(&db) == "value" => Some(expr_id),
            _ => None,
        })
        .expect("value expression");
    assert_eq!(result.expr_ty(body, value_expr), Some(Ty::word(&db)));
}

#[test]
fn storage_string_field_read_loads_as_memory_string_without_context() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data string;
data memory(t) = memory(word);
data storage(t) = storage(word);

forall a b.
class a:CanStore(b) {
  function store(r:a, v:b) -> ();
  function load(r:a) -> b;
}

instance storage(string):CanStore(memory(string)) {
  function store(dst: storage(string), src: memory(string)) -> () {
return ();
  }

  function load(src: storage(string)) -> memory(string) {
return memory(0);
  }
}

contract C {
  value: string;

  function get() {
let x = value;
return x;
  }
}
"#,
    );
    let (body, result) = infer_function(&db, module, "get");
    assert_no_typeck(&result);

    let value_expr = body
        .exprs(&db)
        .iter()
        .find_map(|(expr_id, expr)| match &expr.kind {
            ExprKind::Ident(name) if (*name.atom()).text(&db) == "value" => Some(expr_id),
            _ => None,
        })
        .expect("value expression");
    let string_ty = adt_ty(&db, module, "string", Vec::new());
    let memory_string = adt_ty(&db, module, "memory", vec![string_ty]);
    assert_eq!(result.expr_ty(body, value_expr), Some(memory_string));
}

#[test]
fn storage_mapping_assignment_records_concrete_base_ref_type() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data mapping(index, member) = mapping(word);
data storage(t) = storage(word);

forall a b.
class a:CanStore(b) {
  function store(r:a, v:b) -> ();
  function load(r:a) -> b;
}

instance storage(word):CanStore(word) {
  function store(dst: storage(word), src: word) -> () {
return ();
  }

  function load(src: storage(word)) -> word {
return 0;
  }
}

contract C {
  m: mapping(word, word);

  function next() -> word {
return 1;
  }

  function main() {
m[next()] = next();
  }
}
"#,
    );
    let (body, result) = infer_function(&db, module, "main");
    assert_no_typeck(&result);

    let mapping_expr = body
        .exprs(&db)
        .iter()
        .find_map(|(expr_id, expr)| match &expr.kind {
            ExprKind::Ident(name) if (*name.atom()).text(&db) == "m" => Some(expr_id),
            _ => None,
        })
        .expect("mapping field expression");
    let word = Ty::word(&db);
    let mapping = adt_ty(&db, module, "mapping", vec![word, word]);
    let storage_mapping = adt_ty(&db, module, "storage", vec![mapping]);
    assert_eq!(result.expr_ty(body, mapping_expr), Some(storage_mapping));
}

#[test]
fn constrained_function_call_records_call_site_evidence() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data T = T;

forall a . class a:C {}
instance T:C {}

forall a . a:C => function use(x: a) -> word { return 0; }

function main(t: T) -> word {
  return use(t);
}
"#,
    );
    let info = function_infos(&db, module)
        .into_iter()
        .find(|info| function_name(&db, info.function) == "main")
        .expect("main function");
    let body = info.function.body(&db).expect("main body");
    let call_expr = return_expr(&db, body);
    assert!(matches!(
        body.exprs(&db).get(call_expr).kind,
        ExprKind::Call { .. }
    ));

    let result = infer_all_functions_with_solver(&db, module)
        .into_iter()
        .find(|(name, _)| name == "main")
        .map(|(_, result)| result)
        .expect("main result");

    assert!(
        result.call_site_evidence.iter().any(|evidence| {
            evidence.body == body
                && evidence.call_expr == call_expr
                && matches!(
                    evidence.callee,
                    CallSiteCallee::Function(def)
                        if def.name(&db).as_deref() == Some("use")
                )
        }),
        "expected call-site evidence for use(t), got {:?}",
        result.call_site_evidence
    );
}

#[test]
fn trait_solver_rejects_unproductive_instance_cycle() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:C {}
forall a . a:C => instance a:C {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "C"),
        Ty::word(&db),
        Vec::new(),
    );
    assert!(matches!(solution, Solution::NoSolution));
}

#[test]
fn tabled_solver_cycle_saturates_without_fuel_diagnostic() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:C {}
forall a . a:C => instance a:C {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "C"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(report.solution, Solution::NoSolution));
    assert!(!report.exhausted, "{report:?}");

    let diagnostics = lowered_module_typeck_diagnostics(
        r#"
pragma no-patterson-condition C;

forall a . class a:C {}

forall a . a:C => instance a:C {}

forall a . a:C => function needsC(x:a) -> () {
  return ();
}

function main(x: word) -> () {
  return needsC(x);
}
"#,
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.as_deref() != Some("SC0209")),
        "{diagnostics:?}"
    );
}

#[test]
fn tabled_solver_mutual_recursion_saturates_without_answers() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:C {}
forall a . class a:D {}

forall a . a:D => instance a:C {}
forall a . a:C => instance a:D {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "C"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(report.solution, Solution::NoSolution));
    assert!(!report.exhausted, "{report:?}");
    assert_eq!(report.stats.answers_found, 0, "{report:?}");
}

#[test]
fn tabled_solver_shares_diamond_subgoals() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Leaf {}
forall a . class a:Left {}
forall a . class a:Right {}
forall a . class a:Top {}

instance word:Leaf {}

forall a . a:Leaf => instance a:Left {}
forall a . a:Leaf => instance a:Right {}
forall a . a:Left, a:Right => instance a:Top {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Top"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert!(!report.exhausted, "{report:?}");
    assert_eq!(report.stats.table_size, 4, "{report:?}");
    assert_eq!(report.stats.answers_found, 4, "{report:?}");
}

#[test]
fn tabled_solver_shares_alpha_equivalent_flexible_subgoals() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Pair(a, b) = Pair(a, b);

forall a . class a:Leaf {}
forall a . class a:Left {}
forall a . class a:Right {}
forall a . class a:Top {}

forall a . instance a:Leaf {}

forall a b c . Pair(b, c):Leaf => instance a:Left {}
forall a c b . Pair(b, c):Leaf => instance a:Right {}
forall a . a:Left, a:Right => instance a:Top {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Top"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert!(!report.exhausted, "{report:?}");
    // The Left and Right clauses allocate their two Leaf variables in
    // opposite numeric order, but both conditions canonicalize to one table.
    assert_eq!(report.stats.table_size, 4, "{report:?}");
}

#[test]
fn tabled_solver_dedups_replayed_identical_answer() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Seed {}
forall a . class a:Derived {}

instance word:Seed {}

forall a . a:Seed, a:Seed => instance a:Derived {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Derived"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert_eq!(report.stats.table_size, 2, "{report:?}");
    assert_eq!(report.stats.answers_found, 2, "{report:?}");
}

#[test]
fn tabled_solver_replays_answers_to_late_consumers() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Seed {}
forall a . class a:Derived {}
forall a . class a:Needs {}

instance word:Seed {}

forall a . a:Seed => instance a:Derived {}
forall a . a:Seed, a:Derived => instance a:Needs {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Needs"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert_eq!(report.stats.table_size, 3, "{report:?}");
    assert_eq!(report.stats.answers_found, 3, "{report:?}");
}

#[test]
fn trait_solver_resolves_recursive_pair_instance() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Pair(a, b) = Pair(a, b);

forall a . class a:StorageSize {}

instance word:StorageSize {}

forall a b . a:StorageSize, b:StorageSize => instance Pair(a, b):StorageSize {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let word = Ty::word(&db);
    let pair_word_word = adt_ty(&db, module, "Pair", vec![word, word]);
    let nested = adt_ty(&db, module, "Pair", vec![pair_word_word, word]);

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "StorageSize"),
        nested,
        Vec::new(),
    );

    let Solution::Unique { evidence, .. } = solution else {
        panic!("expected unique solution, got {solution:?}");
    };
    let Evidence::Instance { sub_evidence, .. } = evidence else {
        panic!("expected instance evidence");
    };
    assert_eq!(sub_evidence.len(), 2);
    assert!(matches!(sub_evidence[0], Evidence::Instance { .. }));
    assert!(matches!(sub_evidence[1], Evidence::Instance { .. }));
}

#[test]
fn trait_solver_prefilters_only_heads_that_cannot_unify() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Target {}
forall a . class a:Noise {}
forall a . class a:DefaultTarget {}
forall a . class a:GenericTarget {}
forall a . class a:GivenTarget {}
forall a . class a:Parent {}
forall a . a:Parent => class a:Child {}
forall a . class a:AmbiguousTarget {}

instance word:Target {}
instance bool:Noise {}
forall a . default instance a:Noise {}
forall a . default instance a:DefaultTarget {}
forall a . instance a:GenericTarget {}
instance word:AmbiguousTarget {}
instance word:AmbiguousTarget {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let base_env = trait_env(&db, module, &module_resolution);
    let word = Ty::word(&db);
    let env = trait_env_with_givens(
        &db,
        base_env,
        vec![
            Pred::in_class(&db, class_id(&db, module, "Noise"), word, Vec::new()),
            Pred::in_class(&db, class_id(&db, module, "Child"), word, Vec::new()),
            Pred::in_class(&db, class_id(&db, module, "GivenTarget"), word, Vec::new()),
        ],
    );

    let target = solve_class_report(&db, env, class_id(&db, module, "Target"), word, Vec::new());
    assert!(
        matches!(target.solution, Solution::Unique { .. }),
        "{target:?}"
    );
    assert_eq!(target.stats.generator_steps, 1, "{target:?}");

    let generic = solve_class_report(
        &db,
        env,
        class_id(&db, module, "GenericTarget"),
        word,
        Vec::new(),
    );
    assert!(
        matches!(generic.solution, Solution::Unique { .. }),
        "{generic:?}"
    );
    assert_eq!(generic.stats.generator_steps, 1, "{generic:?}");

    let given = solve_class_report(
        &db,
        env,
        class_id(&db, module, "GivenTarget"),
        word,
        Vec::new(),
    );
    assert!(
        matches!(given.solution, Solution::Unique { .. }),
        "{given:?}"
    );
    assert_eq!(given.stats.generator_steps, 1, "{given:?}");

    let superclass =
        solve_class_report(&db, env, class_id(&db, module, "Parent"), word, Vec::new());
    assert!(
        matches!(superclass.solution, Solution::Unique { .. }),
        "{superclass:?}"
    );
    assert_eq!(superclass.stats.generator_steps, 2, "{superclass:?}");

    let default = solve_class_report(
        &db,
        env,
        class_id(&db, module, "DefaultTarget"),
        Ty::string(&db),
        Vec::new(),
    );
    assert!(
        matches!(default.solution, Solution::Unique { .. }),
        "{default:?}"
    );
    assert_eq!(default.stats.generator_steps, 1, "{default:?}");

    let ambiguous = solve_class_report(
        &db,
        env,
        class_id(&db, module, "AmbiguousTarget"),
        word,
        Vec::new(),
    );
    assert!(
        matches!(ambiguous.solution, Solution::Ambiguous { .. }),
        "{ambiguous:?}"
    );
    assert_eq!(ambiguous.stats.generator_steps, 2, "{ambiguous:?}");
}

#[test]
fn trait_solver_preserves_comptime_transparent_fixed_local_given() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall abs rep . class abs:Typedef(rep) {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let base_env = trait_env(&db, module, &module_resolution);
    let class = class_id(&db, module, "Typedef");
    let context_ty = Ty::bound(&db, 0);
    let env = trait_env_with_givens(
        &db,
        base_env,
        vec![Pred::in_class(&db, class, context_ty, vec![Ty::word(&db)])],
    );
    let goal = Pred::in_class(
        &db,
        class,
        Ty::comptime(&db, context_ty),
        vec![Ty::word(&db)],
    );

    let report = solve_report(&db, env, canonical_goal(&db, goal));

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert_eq!(report.stats.generator_steps, 1, "{report:?}");
}

#[test]
fn trait_solver_preserves_rigid_origin_across_nested_goal_canonicalization() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Wrap(a) = Wrap(a);

forall self rep . class self:Foo(rep) {}
forall self rep . class self:Bar(rep) {}

forall a rep . a:Foo(rep) => instance Wrap(a):Bar(rep) {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let base_env = trait_env(&db, module, &module_resolution);
    let foo = class_id(&db, module, "Foo");
    let bar = class_id(&db, module, "Bar");
    let rigid = Ty::bound(&db, 0);
    let result = Ty::bound(&db, 1);
    let env = trait_env_with_givens(
        &db,
        base_env,
        vec![Pred::in_class(&db, foo, rigid, vec![Ty::word(&db)])],
    );
    let goal = Pred::in_class(
        &db,
        bar,
        adt_ty(&db, module, "Wrap", vec![rigid]),
        vec![result],
    );

    let report = solve_report(
        &db,
        env,
        crate::canonical_goal_with_allowed(&db, goal, vec![1]),
    );

    let Solution::Unique {
        subst,
        evidence: Evidence::Instance { sub_evidence, .. },
    } = &report.solution
    else {
        panic!("expected improved nested solution, got {report:?}");
    };
    assert_eq!(subst.values, vec![(1, Ty::word(&db))]);
    assert!(matches!(
        sub_evidence.as_slice(),
        [Evidence::Builtin { pred }]
            if matches!(
                pred.kind(&db),
                PredKind::InClass { class, main, args }
                    if *class == foo && *main == rigid && args == &vec![Ty::word(&db)]
            )
    ));
    assert!(!report.exhausted, "{report:?}");
}

#[test]
fn inference_improves_multi_parameter_result_through_local_given() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Wrap(a) = Wrap(a);

forall self rep . class self:Foo(rep) {}
forall self rep . class self:Bar(rep) {}

forall a rep . a:Foo(rep) => instance Wrap(a):Bar(rep) {}

forall a rep . Wrap(a):Bar(rep) =>
function need_bar(x:Wrap(a)) -> () {
    return ();
}

forall a . a:Foo(word) =>
function use_bar(x:Wrap(a)) -> () {
    need_bar(x);
    return ();
}
"#,
    );
    let result = infer_all_functions_with_solver(&db, module)
        .into_iter()
        .find_map(|(name, result)| (name == "use_bar").then_some(result))
        .expect("use_bar inference result");

    assert_no_typeck(&result);
    assert!(
        result.call_site_evidence.iter().any(|evidence| {
            matches!(
                evidence.callee,
                CallSiteCallee::Function(def)
                    if def.name(&db).as_deref() == Some("need_bar")
            )
        }),
        "expected solved need_bar call evidence, got {:?}",
        result.call_site_evidence
    );
}

#[test]
fn trait_solver_prefilter_preserves_comptime_correlated_instance_head() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Correlated {}
forall x . instance (comptime x, x):Correlated {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let context_ty = Ty::bound(&db, 0);
    let pair = Ty::named(
        &db,
        TyCtor::Builtin(crate::BuiltinTyCtor::Pair),
        vec![context_ty, context_ty],
    );

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Correlated"),
        pair,
        Vec::new(),
    );

    assert!(
        matches!(report.solution, Solution::Unique { .. }),
        "{report:?}"
    );
    assert_eq!(report.stats.generator_steps, 1, "{report:?}");
}

#[test]
fn trait_solver_prefers_specific_instance_over_default() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Test {}
forall a . default instance a:Test {}
instance word:Test {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let class = class_id(&db, module, "Test");
    let specific = module
        .items(&db)
        .iter()
        .filter_map(|item| match item {
            Item::InstanceDef(instance) if instance.default_kw(&db).is_none() => {
                Some(instance.def_id_value(&db))
            }
            _ => None,
        })
        .next()
        .expect("specific instance");

    let solution = solve_class_goal(&db, env, class, Ty::word(&db), Vec::new());
    let Solution::Unique { evidence, .. } = solution else {
        panic!("expected unique solution, got {solution:?}");
    };
    assert!(matches!(
        evidence,
        Evidence::Instance { instance, .. } if instance == specific
    ));

    let default_solution = solve_class_goal(&db, env, class, Ty::string(&db), Vec::new());
    assert!(matches!(default_solution, Solution::Unique { .. }));
}

#[test]
fn trait_solver_uses_default_instance_for_non_default_clause_condition() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Wrap(a) = Wrap(a);

forall a . class a:DefaultDependency {}
forall a . default instance a:DefaultDependency {}

forall a . class a:Outer {}
forall a . a:DefaultDependency => instance Wrap(a):Outer {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let wrapped_word = adt_ty(&db, module, "Wrap", vec![Ty::word(&db)]);
    let default_dependency = module
        .items(&db)
        .iter()
        .find_map(|item| match item {
            Item::InstanceDef(instance) if instance.default_kw(&db).is_some() => {
                Some(instance.def_id_value(&db))
            }
            _ => None,
        })
        .expect("default dependency instance");

    let report = solve_class_report(
        &db,
        env,
        class_id(&db, module, "Outer"),
        wrapped_word,
        Vec::new(),
    );

    let Solution::Unique { ref evidence, .. } = report.solution else {
        panic!("expected default-backed solution, got {report:?}");
    };
    let Evidence::Instance { sub_evidence, .. } = evidence else {
        panic!("expected outer instance evidence");
    };
    assert_eq!(sub_evidence.len(), 1);
    assert!(matches!(
        &sub_evidence[0],
        Evidence::Instance { instance, .. } if *instance == default_dependency
    ));
    assert!(!report.exhausted, "{report:?}");
    assert_eq!(report.stats.generator_steps, 2, "{report:?}");
}

#[test]
fn trait_solver_reports_overlapping_non_default_instances_as_ambiguous() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:C {}
instance word:C {}
instance word:C {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "C"),
        Ty::word(&db),
        Vec::new(),
    );
    assert!(matches!(
        solution,
        Solution::Ambiguous { candidates } if candidates.len() == 2
    ));
}

#[test]
fn trait_solver_keeps_distinct_substitutions_from_the_same_instance() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Pair(a, b) = Pair(a, b);

forall a r . class a:D(r) {}
forall a . default instance a:D(word) {}
forall a . default instance a:D(bool) {}

forall a . class a:C {}
forall a r . a:D(r) => instance Pair(a, r):C {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let goal = adt_ty(
        &db,
        module,
        "Pair",
        vec![Ty::string(&db), Ty::bound(&db, 0)],
    );

    let goal = Pred::in_class(&db, class_id(&db, module, "C"), goal, Vec::new());
    let solution = solve(
        &db,
        env,
        crate::canonical_goal_with_allowed(&db, goal, vec![0]),
    );

    let Solution::Ambiguous { candidates } = solution else {
        panic!("expected ambiguous same-instance substitutions, got {solution:?}");
    };
    assert_eq!(candidates.len(), 2);
    let substitutions = candidates
        .iter()
        .map(|candidate| candidate.subst.values.clone())
        .collect::<FxHashSet<_>>();
    assert_eq!(substitutions.len(), 2);
}

#[test]
fn trait_solver_unifies_weak_class_args_across_conditions() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
data Uint = Uint(word);

forall abs rep . class abs:Typedef(rep) {}
instance Uint:Typedef(word) {}

forall a . class a:StorageSize {}
instance word:StorageSize {}

forall a b . a:Typedef(b), b:StorageSize => instance a:StorageSize {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let uint = adt_ty(&db, module, "Uint", Vec::new());

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "StorageSize"),
        uint,
        Vec::new(),
    );

    let Solution::Unique { evidence, .. } = solution else {
        panic!("expected weak class argument unification, got {solution:?}");
    };
    let Evidence::Instance { args, .. } = evidence else {
        panic!("expected generic StorageSize instance evidence");
    };
    assert_eq!(args, vec![uint, Ty::word(&db)]);
}

#[test]
fn default_instance_is_blocked_by_unifying_normal_head() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:C {}
instance word:C {}
forall a . default instance a:C {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "C"),
        Ty::bound(&db, 0),
        Vec::new(),
    );

    assert!(matches!(solution, Solution::NoSolution));
}

#[test]
fn imported_class_origin_contributes_superclass_clauses() {
    let mut db = TestDb::default();
    let lib_path = PathBuf::from("/main/lib.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let lib_file = source_file_at_path(
        &db,
        &lib_path,
        r#"
export { Eq, Ord };

forall a . class a:Eq {}
forall a . a:Eq => class a:Ord {}
"#,
    );
    let main_file = source_file_at_path(
        &db,
        &main_path,
        r#"
import lib.{Eq, Ord};

instance word:Ord {}
"#,
    );
    let lib_key = module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &lib_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(lib_key.clone(), lib_file);
    db.insert_module_file(main_key.clone(), main_file);
    let lib_module = module_id_from_key(&db, &lib_key);
    let main_module = module_id_from_key(&db, &main_key);
    let lib_hir = parse_file_to_hir(&db, lib_file).module(&db);

    let env = trait_env_for_module(&db, main_module);
    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, lib_hir, "Eq"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(
        solution,
        Solution::Unique {
            evidence: Evidence::Superclass { .. },
            ..
        }
    ));
    assert_eq!(lib_module.display(&db), "lib");
}

#[test]
fn trait_env_from_module_resolution_and_imports_deduplicates_superclass_modules() {
    let mut db = TestDb::default();
    let lib_path = PathBuf::from("/main/lib.solc");
    let main_path = PathBuf::from("/main/main.solc");
    let lib_file = source_file_at_path(
        &db,
        &lib_path,
        r#"
export { Parent, Child };

forall a . class a:Parent {}
forall a . a:Parent => class a:Child {}
"#,
    );
    let main_file = source_file_at_path(
        &db,
        &main_path,
        r#"
import lib.{Parent, Child};
"#,
    );
    let lib_key = module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &lib_path).unwrap();
    let main_key =
        module_key_for_path(LibraryId::Main, &PathBuf::from("/main"), &main_path).unwrap();
    db.insert_module_file(lib_key, lib_file);
    db.insert_module_file(main_key.clone(), main_file);

    let main_module = module_id_from_key(&db, &main_key);
    let main_hir = parse_file_to_hir(&db, main_file).module(&db);
    let imports = nameres::module_env_for_hir_module(&db, main_module, main_hir);
    let item_scope = imports.item_scope.clone().expect("main item scope");
    let resolution = hir_nameres::resolve_module_with_imports(&db, main_hir, item_scope, &imports);
    let trait_env =
        trait_env_from_module_resolution_and_imports(&db, main_hir, &resolution, &imports);
    let ClassId::User(child_def) =
        class_id(&db, parse_file_to_hir(&db, lib_file).module(&db), "Child")
    else {
        panic!("Child must be a user-defined class");
    };
    let superclass_origins = trait_env
        .clauses(&db)
        .iter()
        .filter_map(|clause| match &clause.origin {
            ClauseOrigin::Superclass(def) => Some(*def),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(superclass_origins, vec![child_def]);
}

#[test]
fn superclass_solution_records_projection_evidence() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Eq {}
forall a . a:Eq => class a:Ord {}
instance word:Ord {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "Eq"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(
        solution,
        Solution::Unique {
            evidence: Evidence::Superclass {
                child,
                ..
            },
            ..
        } if matches!(*child, Evidence::Instance { .. })
    ));
}

#[test]
fn direct_instance_precedes_superclass_projection() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Eq {}
forall a . a:Eq => class a:Ord {}
instance word:Eq {}
instance word:Ord {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "Eq"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(
        solution,
        Solution::Unique {
            evidence: Evidence::Instance { .. },
            ..
        }
    ));
}

#[test]
fn local_givens_and_superclasses_precede_global_instances() {
    let db = TestDb::default();
    let module = parse_module(
        &db,
        r#"
forall a . class a:Eq {}
forall a . a:Eq => class a:Ord {}
instance word:Eq {}
"#,
    );
    let module_resolution = hir_nameres::resolve_module(&db, module);
    let env = trait_env(&db, module, &module_resolution);
    let env = trait_env_with_givens(
        &db,
        env,
        vec![Pred::in_class(
            &db,
            class_id(&db, module, "Ord"),
            Ty::word(&db),
            Vec::new(),
        )],
    );

    let solution = solve_class_goal(
        &db,
        env,
        class_id(&db, module, "Eq"),
        Ty::word(&db),
        Vec::new(),
    );

    assert!(matches!(
        solution,
        Solution::Unique {
            evidence: Evidence::Superclass {
                child,
                ..
            },
            ..
        } if matches!(*child, Evidence::Builtin { .. })
    ));
}

#[test]
fn pragma_corpus_files_have_no_instance_soundness_diagnostics() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let corpus = manifest.join("../parser/tests/fixtures/corpus");
    let files = [
        "pragmas/coverage.solc",
        "cases/array.solc",
        "cases/bound-with-pragma.solc",
        "cases/tabled-left-recursive-fail.solc",
        "cases/tabled-cycle-fail.solc",
        "cases/mptc-partial-instance.solc",
    ];

    for file in files {
        let path = ["ok", "fail"]
            .into_iter()
            .map(|status| corpus.join(status).join("test/examples").join(file))
            .find(|path| path.exists())
            .expect("corpus fixture");
        let src = std::fs::read_to_string(path).expect("fixture source");
        let (db, key) = db_with_main_typeck(&src);
        let source = *db.module_files.get(&key).expect("main source");
        assert!(
            parser::parse_diagnostics(&db, source).is_empty(),
            "{file} should parse cleanly"
        );
        let module_id = module_id_from_key(&db, &key);
        let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module_id).clone();
        assert!(
            diagnostics.is_empty(),
            "{file} produced instance soundness diagnostics: {diagnostics:?}"
        );
    }
}

#[test]
fn structured_default_instance_head_is_allowed_only_when_it_contains_a_type_variable() {
    let (db, key) = db_with_main_typeck(
        r#"
data Box(a) = Box(a);
forall a . class a:Marker {}
forall a . default instance Box(a):Marker {}
"#,
    );
    let module_id = module_id_from_key(&db, &key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module_id);
    assert!(
        diagnostics.iter().all(|diagnostic| !matches!(
            diagnostic,
            TypeckDiagnostic::InvalidDefaultInstance { .. }
        )),
        "{diagnostics:?}"
    );

    let (db, key) = db_with_main_typeck(
        r#"
data Box(a) = Box(a);
forall a . class a:Marker {}
default instance Box(word):Marker {}
"#,
    );
    let module_id = module_id_from_key(&db, &key);
    let diagnostics = crate::solver::instance_soundness_diagnostics(&db, module_id);
    assert!(
        diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            TypeckDiagnostic::InvalidDefaultInstance { .. }
        )),
        "{diagnostics:?}"
    );
}
