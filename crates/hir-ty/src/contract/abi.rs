use hir::{
    anchor::DefId,
    ast::item::{AdtDef, ContractItem, Item, Module},
    diag::Diagnostic,
    nameres as hir_nameres,
};
use nameres::{LibraryId, module_id_for_source_file};
use parser::parse_file_to_hir;
use rustc_hash::FxHashSet;

use crate::{
    BuiltinTyCtor, Db, Ty, TyCtor, TyKind, UserTyCtor, support::is_canonical_std_def_named,
};

/// ABI parameter or tuple component.
#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub struct AbiParam {
    /// Parameter name. Outputs and tuple components use the empty name,
    /// matching the reference ABI emitter.
    pub name: String,
    /// Canonical ABI type.
    pub ty: AbiType,
    /// Tuple components, if `ty` is `AbiType::Tuple`.
    pub components: Vec<AbiParam>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, salsa::Update)]
pub enum AbiType {
    Uint256,
    Bool,
    String,
    Unit,
    Tuple,
    Named(String),
    Unsupported,
}

impl std::fmt::Display for AbiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AbiType::Uint256 => f.write_str("uint256"),
            AbiType::Bool => f.write_str("bool"),
            AbiType::String => f.write_str("string"),
            AbiType::Unit => Ok(()),
            AbiType::Tuple => f.write_str("tuple"),
            AbiType::Named(name) => f.write_str(name),
            AbiType::Unsupported => f.write_str("<unsupported>"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::Update)]
pub struct AbiSelector(pub [u8; 4]);

impl AbiSelector {
    pub fn to_hex(self) -> String {
        format!(
            "0x{:02x}{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

/// Interned ABI signature preimage used as the selector query key.
#[salsa::interned(debug)]
pub struct AbiSignature<'db> {
    /// Canonical signature, e.g. `transfer(address,uint256)`.
    #[returns(ref)]
    pub text: String,
}

/// Computes the ABI selector for a canonical signature.
#[salsa::tracked]
pub fn abi_selector<'db>(db: &'db dyn Db, signature: AbiSignature<'db>) -> AbiSelector {
    let hash = hir::keccak::keccak256(signature.text(db).as_bytes());
    AbiSelector([hash[0], hash[1], hash[2], hash[3]])
}

pub(super) fn method_signature_string<'db>(
    db: &'db dyn Db,
    name: &str,
    params: &[Ty<'db>],
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str(name);
    out.push('(');
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&signature_type_string(
            db,
            *param,
            &mut Vec::new(),
            visible_manual_generic_adts,
        )?);
    }
    out.push(')');
    Ok(out)
}

fn signature_type_string<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    match ty.kind(db) {
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Word),
            args,
        } if args.is_empty() => Ok(AbiType::Uint256.to_string()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Bool),
            args,
        } if args.is_empty() => Ok(AbiType::Bool.to_string()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::String),
            args,
        } if args.is_empty() => Ok(AbiType::String.to_string()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Unit),
            args,
        } if args.is_empty() => Ok(AbiType::Unit.to_string()),
        TyKind::Tuple(elems) => {
            tuple_signature_string(db, elems, adt_stack, visible_manual_generic_adts)
        }
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } if args.len() == 2 => {
            tuple_signature_string(db, args, adt_stack, visible_manual_generic_adts)
        }
        TyKind::Named {
            ctor: TyCtor::User(user),
            args,
        } => {
            if args.is_empty()
                && let Some(name) = canonical_user_abi_name(db, user)
            {
                return Ok(name);
            }
            if let Some(element) = canonical_calldata_array_element(db, user, args)? {
                return Ok(format!(
                    "{}[]",
                    generic_sig_string(db, element, adt_stack, visible_manual_generic_adts,)?
                ));
            }
            if let Some(name) = canonical_location_abi_name(db, user, args)? {
                return Ok(name);
            }
            reject_structural_std_abi_fallback(db, user, args)?;
            Err(unsupported_user_adt_abi_type(db, user, args))
        }
        TyKind::Error | TyKind::Unknown | TyKind::BoundVar(_) => Err(ty.display(db)),
        TyKind::Named { .. } | TyKind::Function { .. } | TyKind::Comptime(_) => Err(ty.display(db)),
    }
}

fn tuple_signature_string<'db>(
    db: &'db dyn Db,
    elems: &[Ty<'db>],
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    let mut parts = Vec::new();
    for elem in flatten_tuple(db, elems) {
        parts.push(signature_type_string(
            db,
            elem,
            adt_stack,
            visible_manual_generic_adts,
        )?);
    }
    Ok(format!("({})", parts.join(",")))
}

pub(super) fn abi_params<'db>(
    db: &'db dyn Db,
    names: &[String],
    tys: &[Ty<'db>],
    diagnostics: &mut Vec<Diagnostic>,
    span: hir::span::Span<'db>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Vec<AbiParam> {
    tys.iter()
        .enumerate()
        .map(|(index, ty)| {
            match abi_param(
                db,
                names.get(index).cloned().unwrap_or_default(),
                *ty,
                &mut Vec::new(),
                visible_manual_generic_adts,
            ) {
                Ok(param) => param,
                Err(err) => {
                    diagnostics.push(contract_diag_unsupported_abi_type(
                        db,
                        span,
                        "ABI parameter",
                        &err,
                    ));
                    AbiParam {
                        name: names.get(index).cloned().unwrap_or_default(),
                        ty: AbiType::Unsupported,
                        components: Vec::new(),
                    }
                }
            }
        })
        .collect()
}

pub(super) fn abi_outputs<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    diagnostics: &mut Vec<Diagnostic>,
    span: hir::span::Span<'db>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Vec<AbiParam> {
    if is_unit_ty(db, ty) {
        return Vec::new();
    }
    flatten_output_ty(db, ty)
        .into_iter()
        .map(|ty| {
            if contains_canonical_calldata_array(db, ty) {
                diagnostics.push(contract_diag_unsupported_abi_type(
                    db,
                    span,
                    "ABI output",
                    &format!(
                        "{} (calldata(array(t)) is input-only; the target std has no ABIEncode evidence for it)",
                        ty.display(db)
                    ),
                ));
                return AbiParam {
                    name: String::new(),
                    ty: AbiType::Unsupported,
                    components: Vec::new(),
                };
            }
            match abi_param(
                db,
                String::new(),
                ty,
                &mut Vec::new(),
                visible_manual_generic_adts,
            ) {
                Ok(param) => param,
                Err(err) => {
                    diagnostics.push(contract_diag_unsupported_abi_type(
                        db,
                        span,
                        "ABI output",
                        &err,
                    ));
                    AbiParam {
                        name: String::new(),
                        ty: AbiType::Unsupported,
                        components: Vec::new(),
                    }
                }
            }
        })
        .collect()
}

fn abi_param<'db>(
    db: &'db dyn Db,
    name: String,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<AbiParam, String> {
    let (ty, components) = abi_type_of(db, ty, adt_stack, visible_manual_generic_adts)?;
    Ok(AbiParam {
        name,
        ty,
        components,
    })
}

#[derive(Clone, Copy)]
enum UserAdtMode {
    Reject,
    DerivedName,
}

fn abi_param_with_adt_mode<'db>(
    db: &'db dyn Db,
    name: String,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    user_adt_mode: UserAdtMode,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<AbiParam, String> {
    let (ty, components) = abi_type_of_with_adt_mode(
        db,
        ty,
        adt_stack,
        user_adt_mode,
        visible_manual_generic_adts,
    )?;
    Ok(AbiParam {
        name,
        ty,
        components,
    })
}

fn abi_type_of<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<(AbiType, Vec<AbiParam>), String> {
    abi_type_of_with_adt_mode(
        db,
        ty,
        adt_stack,
        UserAdtMode::Reject,
        visible_manual_generic_adts,
    )
}

fn abi_type_of_with_adt_mode<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    user_adt_mode: UserAdtMode,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<(AbiType, Vec<AbiParam>), String> {
    match ty.kind(db) {
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Word),
            args,
        } if args.is_empty() => Ok((AbiType::Uint256, Vec::new())),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Bool),
            args,
        } if args.is_empty() => Ok((AbiType::Bool, Vec::new())),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::String),
            args,
        } if args.is_empty() => Ok((AbiType::String, Vec::new())),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Unit),
            args,
        } if args.is_empty() => Ok((AbiType::Unit, Vec::new())),
        TyKind::Tuple(elems) if elems.is_empty() => Ok((AbiType::Unit, Vec::new())),
        TyKind::Tuple(elems) => Ok((
            AbiType::Tuple,
            flatten_tuple(db, elems)
                .into_iter()
                .map(|elem| {
                    abi_param_with_adt_mode(
                        db,
                        String::new(),
                        elem,
                        adt_stack,
                        user_adt_mode,
                        visible_manual_generic_adts,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } if args.len() == 2 => Ok((
            AbiType::Tuple,
            flatten_tuple(db, args)
                .into_iter()
                .map(|elem| {
                    abi_param_with_adt_mode(
                        db,
                        String::new(),
                        elem,
                        adt_stack,
                        user_adt_mode,
                        visible_manual_generic_adts,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        TyKind::Named {
            ctor: TyCtor::User(user),
            args,
        } => {
            if args.is_empty()
                && let Some(name) = canonical_user_abi_name(db, user)
            {
                return Ok((AbiType::Named(name), Vec::new()));
            }
            if let Some(element) = canonical_calldata_array_element(db, user, args)? {
                let (element_ty, components) = abi_type_of_with_adt_mode(
                    db,
                    element,
                    adt_stack,
                    UserAdtMode::DerivedName,
                    visible_manual_generic_adts,
                )?;
                return Ok((AbiType::Named(format!("{element_ty}[]")), components));
            }
            if let Some(name) = canonical_location_abi_name(db, user, args)? {
                return Ok((AbiType::Named(name), Vec::new()));
            }
            reject_structural_std_abi_fallback(db, user, args)?;
            if matches!(user_adt_mode, UserAdtMode::DerivedName) {
                let name = compiler_owned_generic_source_name(
                    db,
                    user,
                    args,
                    adt_stack,
                    visible_manual_generic_adts,
                )?;
                return Ok((AbiType::Named(name), Vec::new()));
            }
            Err(unsupported_user_adt_abi_type(db, user, args))
        }
        _ => Err(ty.display(db)),
    }
}

/// Returns the element of the one externally supported calldata location:
/// `calldata(array(t))`. Both wrappers must be the canonical definitions from
/// `std`; same-named user ADTs must not acquire ABI meaning by spelling alone.
fn canonical_calldata_array_element<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
) -> Result<Option<Ty<'db>>, String> {
    if !is_canonical_std_def_named(db, user.def, "calldata") {
        return Ok(None);
    }
    let [inner] = args else {
        return Err(format!(
            "calldata (expected one type argument, found {})",
            args.len()
        ));
    };
    let TyKind::Named {
        ctor: TyCtor::User(array),
        args: elements,
    } = inner.kind(db)
    else {
        return Err(format!(
            "{} (only calldata(array(t)) has canonical external ABI evidence)",
            inner.display(db)
        ));
    };
    if !is_canonical_std_def_named(db, array.def, "array") {
        return Err(format!(
            "{} (only the canonical std array supports calldata ABI decoding)",
            inner.display(db)
        ));
    }
    let [element] = elements.as_slice() else {
        return Err(format!(
            "array (expected one type argument, found {})",
            elements.len()
        ));
    };
    Ok(Some(*element))
}

/// Mirrors the final `SigString` produced by the target standard library.
/// Products are comma-joined without adding a tuple wrapper, sums retain their
/// explicit `sum(l,r)` wrapper, and automatically derived ADTs recurse through
/// their compiler-owned `Generic` representation.
fn generic_sig_string<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    match ty.kind(db) {
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Word),
            args,
        } if args.is_empty() => Ok("uint256".to_owned()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Bool),
            args,
        } if args.is_empty() => Ok("bool".to_owned()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::String),
            args,
        } if args.is_empty() => Ok("string".to_owned()),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Unit),
            args,
        } if args.is_empty() => Ok(String::new()),
        TyKind::Tuple(elems) => {
            generic_product_sig_string(db, elems, adt_stack, visible_manual_generic_adts)
        }
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } if args.len() == 2 => {
            generic_product_sig_string(db, args, adt_stack, visible_manual_generic_adts)
        }
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Sum),
            args,
        } if args.len() == 2 => Ok(format!(
            "sum({},{})",
            generic_sig_string(db, args[0], adt_stack, visible_manual_generic_adts)?,
            generic_sig_string(db, args[1], adt_stack, visible_manual_generic_adts)?
        )),
        TyKind::Named {
            ctor: TyCtor::User(user),
            args,
        } => {
            if args.is_empty()
                && let Some(name) = canonical_user_abi_name(db, user)
            {
                return Ok(name);
            }
            if let Some(element) = canonical_calldata_array_element(db, user, args)? {
                return Ok(format!(
                    "{}[]",
                    generic_sig_string(db, element, adt_stack, visible_manual_generic_adts,)?
                ));
            }
            if let Some(name) = canonical_location_abi_name(db, user, args)? {
                return Ok(name);
            }
            reject_structural_std_abi_fallback(db, user, args)?;
            compiler_owned_generic_sig_string(
                db,
                user,
                args,
                adt_stack,
                visible_manual_generic_adts,
            )
        }
        TyKind::Error | TyKind::Unknown | TyKind::BoundVar(_) => Err(ty.display(db)),
        TyKind::Named { .. } | TyKind::Function { .. } | TyKind::Comptime(_) => Err(ty.display(db)),
    }
}

fn generic_product_sig_string<'db>(
    db: &'db dyn Db,
    elems: &[Ty<'db>],
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    elems
        .iter()
        .map(|elem| generic_sig_string(db, *elem, adt_stack, visible_manual_generic_adts))
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join(","))
}

fn compiler_owned_generic_source_name<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    compiler_owned_generic_sig_string(db, user, args, adt_stack, visible_manual_generic_adts)?;
    Ok(user
        .def
        .name(db)
        .unwrap_or_else(|| "<anonymous ADT>".to_owned()))
}

fn compiler_owned_generic_sig_string<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
    adt_stack: &mut Vec<DefId<'db>>,
    visible_manual_generic_adts: &FxHashSet<DefId<'db>>,
) -> Result<String, String> {
    let name = user
        .def
        .name(db)
        .unwrap_or_else(|| "<anonymous ADT>".to_owned());
    if !args.is_empty() {
        return Err(format!(
            "{} (parameterized user ADTs are not supported in calldata array ABIs)",
            crate::display::display_ty_source(
                db,
                Ty::named(db, TyCtor::User(*user), args.to_vec()),
                &[]
            )
        ));
    }
    if adt_stack.contains(&user.def) {
        return Err(format!(
            "{name} (recursive ADTs have no finite calldata array ABI signature)"
        ));
    }
    let module = parse_file_to_hir(db, user.def.file(db)).module(db);
    if visible_manual_generic_adts.contains(&user.def)
        || generic_derivation_is_excluded(db, module, &name)
        || has_local_manual_generic_instance(db, module, user.def)
    {
        return Err(format!(
            "{name} (manual or excluded Generic representations are not canonical ABI layouts)"
        ));
    }
    let adt = find_adt_by_def(db, module, user.def)
        .ok_or_else(|| format!("{name} (definition is unavailable for ABI lowering)"))?;
    let plan = crate::solver::derived_generic_plan(db, module, adt)
        .ok_or_else(|| format!("{name} (cannot derive its Generic ABI representation)"))?;

    adt_stack.push(user.def);
    let result = generic_sig_string(db, plan.rep, adt_stack, visible_manual_generic_adts);
    adt_stack.pop();
    result
}

fn has_local_manual_generic_instance<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    adt: DefId<'db>,
) -> bool {
    let item_resolutions = abi_item_type_resolutions(db, module);
    module.items(db).iter().any(|item| {
        let Item::InstanceDef(instance) = item else {
            return false;
        };
        let head = instance.head(db);
        let class_is_generic = item_resolutions.preds.iter().any(|entry| {
            entry.pred == head
                && matches!(
                    &entry.resolution,
                    hir_nameres::Resolution::Def {
                        def,
                        kind: hir_nameres::DefResolutionKind::Class,
                    } if def.name(db).as_deref() == Some("Generic")
                )
        });
        if !class_is_generic {
            return false;
        }
        let main = head.kind(db).ty;
        item_resolutions.types.iter().any(|entry| {
            entry.ty == main
                && matches!(
                    &entry.resolution,
                    hir_nameres::Resolution::Def {
                        def,
                        kind: hir_nameres::DefResolutionKind::Adt,
                    } if *def == adt
                )
        })
    })
}

fn abi_item_type_resolutions<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
) -> hir_nameres::ItemResolutionFacts<'db> {
    let Some(module_id) = module_id_for_source_file(db, module.def_id_value(db).file(db)) else {
        return hir_nameres::resolve_item_type_facts(db, module);
    };
    let env = nameres::module_import_surface(db, module_id);
    let Some(scope) = env.item_scope.as_ref() else {
        return hir_nameres::resolve_item_type_facts(db, module);
    };
    hir_nameres::resolve_item_type_facts_with_imports(db, module, scope, &env)
}

fn canonical_user_abi_name(db: &dyn Db, user: &UserTyCtor<'_>) -> Option<String> {
    let name = user.def.name(db)?;
    if !is_canonical_std_def_named(db, user.def, &name) {
        return None;
    }
    match name.as_str() {
        "uint256" | "address" | "bytes4" | "bytes32" => Some(name),
        _ => None,
    }
}

fn contains_canonical_calldata_array(db: &dyn Db, ty: Ty<'_>) -> bool {
    match ty.kind(db) {
        TyKind::Named {
            ctor: TyCtor::User(user),
            args,
        } => {
            matches!(
                canonical_calldata_array_element(db, user, args),
                Ok(Some(_))
            ) || args
                .iter()
                .any(|arg| contains_canonical_calldata_array(db, *arg))
        }
        TyKind::Named { args, .. } | TyKind::Tuple(args) => args
            .iter()
            .any(|arg| contains_canonical_calldata_array(db, *arg)),
        TyKind::Function { params, ret } => {
            params
                .iter()
                .any(|param| contains_canonical_calldata_array(db, *param))
                || contains_canonical_calldata_array(db, *ret)
        }
        TyKind::Comptime(inner) => contains_canonical_calldata_array(db, *inner),
        TyKind::Error | TyKind::Unknown | TyKind::BoundVar(_) => false,
    }
}

fn canonical_location_abi_name<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
) -> Result<Option<String>, String> {
    let Some(name) = user.def.name(db) else {
        return Ok(None);
    };
    if !matches!(name.as_str(), "memory" | "calldata" | "storage" | "mapping")
        || !is_canonical_std_def_named(db, user.def, &name)
    {
        return Ok(None);
    }
    if name != "memory" {
        return Err(format!(
            "{} ({name} values are not supported by the canonical external ABI)",
            Ty::named(db, TyCtor::User(*user), args.to_vec()).display(db)
        ));
    }
    let [inner] = args else {
        return Err(format!(
            "memory (expected one type argument, found {})",
            args.len()
        ));
    };
    let TyKind::Named {
        ctor: TyCtor::User(inner_user),
        args: inner_args,
    } = inner.kind(db)
    else {
        return Err(format!(
            "{} (only memory(string) and memory(bytes) have canonical ABI evidence)",
            inner.display(db)
        ));
    };
    if !inner_args.is_empty() {
        return Err(format!(
            "{} (only memory(string) and memory(bytes) have canonical ABI evidence)",
            inner.display(db)
        ));
    }
    let Some(inner_name) = inner_user.def.name(db) else {
        return Ok(None);
    };
    if matches!(inner_name.as_str(), "string" | "bytes")
        && is_canonical_std_def_named(db, inner_user.def, &inner_name)
    {
        return Ok(Some(inner_name));
    }
    Err(format!(
        "{} (only memory(string) and memory(bytes) have canonical ABI evidence)",
        inner.display(db)
    ))
}

fn reject_structural_std_abi_fallback<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
) -> Result<(), String> {
    let is_std = module_id_for_source_file(db, user.def.file(db))
        .is_some_and(|module| module.library(db) == &LibraryId::Std);
    if !is_std {
        return Ok(());
    }
    let name = user
        .def
        .name(db)
        .unwrap_or_else(|| "<anonymous standard-library type>".to_owned());
    Err(format!(
        "{} (standard-library type `{name}` has no canonical external ABI evidence)",
        Ty::named(db, TyCtor::User(*user), args.to_vec()).display(db)
    ))
}

fn unsupported_user_adt_abi_type<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
) -> String {
    let ty = Ty::named(db, TyCtor::User(*user), args.to_vec());
    format!(
        "{} (user-defined ADTs are not supported by the canonical external ABI)",
        crate::display::display_ty_source(db, ty, &[])
    )
}

fn user_adt_product_fields<'db>(
    db: &'db dyn Db,
    user: &UserTyCtor<'db>,
    args: &[Ty<'db>],
    adt_stack: &mut Vec<DefId<'db>>,
) -> Result<Vec<Ty<'db>>, String> {
    if adt_stack.contains(&user.def) {
        return Err(format!(
            "{} (recursive ADTs have no finite canonical ABI tuple)",
            user.def
                .name(db)
                .unwrap_or_else(|| "<anonymous ADT>".to_owned())
        ));
    }
    let module = parse_file_to_hir(db, user.def.file(db)).module(db);
    let name = user
        .def
        .name(db)
        .unwrap_or_else(|| "<anonymous ADT>".to_owned());
    if generic_derivation_is_excluded(db, module, &name) {
        return Err(format!(
            "{name} (manual or excluded Generic representations are not canonical ABI layouts)"
        ));
    }
    let adt = find_adt_by_def(db, module, user.def).ok_or_else(|| {
        format!(
            "{} (definition is unavailable for ABI lowering)",
            user.def
                .name(db)
                .unwrap_or_else(|| "<anonymous ADT>".to_owned())
        )
    })?;
    let ctor = match adt.ctors(db).as_slice() {
        [ctor] => ctor,
        [] => {
            return Err(format!(
                "{name} (constructorless ADTs have no canonical ABI representation)"
            ));
        }
        [_, _, ..] => {
            return Err(format!(
                "{name} (multi-constructor ADTs have no canonical ABI representation)"
            ));
        }
    };
    if ctor.field_count == 0 {
        return Err(format!(
            "{name} (zero-field ADTs have no canonical ABI tuple representation)"
        ));
    }
    if adt.ty_param_elems(db).len() != args.len() {
        return Err(format!(
            "{name} (expected {} ABI type arguments, found {})",
            adt.ty_param_elems(db).len(),
            args.len()
        ));
    }
    let plan = crate::solver::derived_generic_plan(db, module, adt)
        .ok_or_else(|| format!("{name} (cannot derive its Generic ABI representation)"))?;
    let product_rep = plan
        .from_arms
        .first()
        .map(|arm| substitute_bound_tys(db, arm.product_rep, args))
        .ok_or_else(|| format!("{name} (cannot derive its constructor product representation)"))?;
    let fields = split_constructor_product(db, product_rep, ctor.field_count).ok_or_else(|| {
        format!(
            "{name} (its Generic product does not match the source constructor arity {})",
            ctor.field_count
        )
    })?;
    if fields.iter().any(|field| is_abi_tuple_shape(db, *field)) {
        return Err(format!(
            "{name} (tuple-typed constructor fields are unsupported because Generic erases their ABI tuple boundary)"
        ));
    }
    adt_stack.push(user.def);
    Ok(fields)
}

fn generic_derivation_is_excluded(db: &dyn Db, module: Module<'_>, adt_name: &str) -> bool {
    module.items(db).iter().any(|item| {
        let Item::Pragma(pragma) = item else {
            return false;
        };
        (*pragma.name(db).atom()).text(db) == "no-generic-instance-for"
            && pragma
                .items(db)
                .iter()
                .any(|item| (*item.atom()).text(db) == adt_name)
    })
}

fn find_adt_by_def<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    def: DefId<'db>,
) -> Option<AdtDef<'db>> {
    module.items(db).iter().find_map(|item| match item {
        Item::AdtDef(adt) if adt.def_id_value(db) == def => Some(*adt),
        Item::ContractDef(contract) => contract.items(db).iter().find_map(|item| match item {
            ContractItem::AdtDef(adt) if adt.def_id_value(db) == def => Some(*adt),
            _ => None,
        }),
        _ => None,
    })
}

fn substitute_bound_tys<'db>(db: &'db dyn Db, ty: Ty<'db>, args: &[Ty<'db>]) -> Ty<'db> {
    match ty.kind(db) {
        TyKind::BoundVar(var) => args.get(var.index as usize).copied().unwrap_or(ty),
        TyKind::Named { ctor, args: inner } => Ty::named(
            db,
            *ctor,
            inner
                .iter()
                .map(|ty| substitute_bound_tys(db, *ty, args))
                .collect(),
        ),
        TyKind::Function { params, ret } => Ty::function(
            db,
            params
                .iter()
                .map(|ty| substitute_bound_tys(db, *ty, args))
                .collect(),
            substitute_bound_tys(db, *ret, args),
        ),
        TyKind::Tuple(elems) => Ty::tuple(
            db,
            elems
                .iter()
                .map(|ty| substitute_bound_tys(db, *ty, args))
                .collect(),
        ),
        TyKind::Comptime(inner) => Ty::comptime(db, substitute_bound_tys(db, *inner, args)),
        TyKind::Error | TyKind::Unknown => ty,
    }
}

fn split_constructor_product<'db>(
    db: &'db dyn Db,
    mut product: Ty<'db>,
    arity: usize,
) -> Option<Vec<Ty<'db>>> {
    if arity == 0 {
        return None;
    }
    let mut fields = Vec::with_capacity(arity);
    for _ in 1..arity {
        let TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } = product.kind(db)
        else {
            return None;
        };
        if args.len() != 2 {
            return None;
        }
        fields.push(args[0]);
        product = args[1];
    }
    fields.push(product);
    Some(fields)
}

fn is_abi_tuple_shape(db: &dyn Db, ty: Ty<'_>) -> bool {
    match ty.kind(db) {
        TyKind::Tuple(_) => true,
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } => args.len() == 2,
        _ => false,
    }
}

pub(super) fn abi_type_contains_user_adt<'db>(
    db: &'db dyn Db,
    ty: Ty<'db>,
    target: DefId<'db>,
) -> bool {
    fn visit<'db>(
        db: &'db dyn Db,
        ty: Ty<'db>,
        target: DefId<'db>,
        adt_stack: &mut Vec<DefId<'db>>,
    ) -> bool {
        match ty.kind(db) {
            TyKind::Tuple(elems) => elems.iter().any(|elem| visit(db, *elem, target, adt_stack)),
            TyKind::Named {
                ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
                args,
            } if args.len() == 2 => args.iter().any(|elem| visit(db, *elem, target, adt_stack)),
            TyKind::Named {
                ctor: TyCtor::User(user),
                args,
            } if is_canonical_std_abi_container(db, user) && args.len() == 1 => {
                visit(db, args[0], target, adt_stack)
            }
            TyKind::Named {
                ctor: TyCtor::User(user),
                args,
            } if args.is_empty() && canonical_user_abi_name(db, user).is_some() => false,
            TyKind::Named {
                ctor: TyCtor::User(user),
                args,
            } => {
                if user.def == target {
                    return true;
                }
                let Ok(fields) = user_adt_product_fields(db, user, args, adt_stack) else {
                    return false;
                };
                let found = fields
                    .into_iter()
                    .any(|field| visit(db, field, target, adt_stack));
                adt_stack.pop();
                found
            }
            TyKind::Comptime(inner) => visit(db, *inner, target, adt_stack),
            TyKind::Error
            | TyKind::Unknown
            | TyKind::BoundVar(_)
            | TyKind::Named { .. }
            | TyKind::Function { .. } => false,
        }
    }

    visit(db, ty, target, &mut Vec::new())
}

fn flatten_output_ty<'db>(db: &'db dyn Db, ty: Ty<'db>) -> Vec<Ty<'db>> {
    match ty.kind(db) {
        TyKind::Tuple(elems) => flatten_tuple(db, elems),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
            args,
        } if args.len() == 2 => flatten_tuple(db, args),
        _ => vec![ty],
    }
}

fn flatten_tuple<'db>(db: &'db dyn Db, elems: &[Ty<'db>]) -> Vec<Ty<'db>> {
    let mut out = Vec::new();
    for elem in elems {
        match elem.kind(db) {
            TyKind::Tuple(nested) => out.extend(flatten_tuple(db, nested)),
            TyKind::Named {
                ctor: TyCtor::Builtin(BuiltinTyCtor::Pair),
                args,
            } if args.len() == 2 => out.extend(flatten_tuple(db, args)),
            _ => out.push(*elem),
        }
    }
    out
}

fn is_unit_ty<'db>(db: &'db dyn Db, ty: Ty<'db>) -> bool {
    matches!(
        ty.kind(db),
        TyKind::Tuple(elems) if elems.is_empty()
    ) || matches!(
        ty.kind(db),
        TyKind::Named {
            ctor: TyCtor::Builtin(BuiltinTyCtor::Unit),
            args,
        } if args.is_empty()
    )
}

fn is_canonical_std_location(db: &dyn Db, user: &UserTyCtor<'_>) -> bool {
    user.def.name(db).is_some_and(|name| {
        matches!(name.as_str(), "memory" | "calldata" | "storage" | "mapping")
            && is_canonical_std_def_named(db, user.def, &name)
    })
}

fn is_canonical_std_abi_container(db: &dyn Db, user: &UserTyCtor<'_>) -> bool {
    is_canonical_std_location(db, user) || is_canonical_std_def_named(db, user.def, "array")
}

pub(super) fn contract_diag_unsupported_abi_type<'db>(
    db: &'db dyn Db,
    span: hir::span::Span<'db>,
    context: &str,
    ty: &str,
) -> Diagnostic {
    Diagnostic::error(format!("{context} cannot be represented in the ABI: {ty}"))
        .with_code("SC0231")
        .with_primary_label(db, span, Some("unsupported ABI type"))
}
