use super::derived_generic::AdtDeriveInfo;
use super::*;

/// Definitions needed to synthesize the ABI instances backed by a compiler-
/// owned `Generic` representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, salsa::Update)]
pub(super) struct DerivedAbiClauseSource<'db> {
    /// Marker exported by `std.ABIGeneric`.
    pub marker: DefId<'db>,
    /// `ABIAttribs` class.
    pub abi_attribs: DefId<'db>,
    /// `ABIDecode` class.
    pub abi_decode: DefId<'db>,
    /// `WordReader` class.
    pub word_reader: DefId<'db>,
    /// `ABIDecoder(ty, reader)` data type.
    pub abi_decoder: DefId<'db>,
}

pub(super) fn visible_abi_clause_source<'db>(
    db: &'db dyn Db,
    env: &nameres::ModuleImportSurface<'db>,
) -> Option<DerivedAbiClauseSource<'db>> {
    let lookup = |name: &str| {
        env.types
            .get(name)
            .and_then(|resolution| def_from_resolution_named(db, resolution, name))
            .or_else(|| {
                env.item_scope
                    .as_ref()
                    .and_then(|scope| scope.types.get(name))
                    .and_then(|entry| def_from_resolution_named(db, &entry.resolution, name))
            })
    };
    abi_clause_source_from_lookup(db, lookup)
}

pub(super) fn resolved_abi_clause_source<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    item_resolutions: &hir_nameres::ItemResolutionFacts<'db>,
) -> Option<DerivedAbiClauseSource<'db>> {
    let lookup = |name: &str| {
        local_named_def(db, module, name).or_else(|| {
            item_resolutions
                .preds
                .iter()
                .map(|entry| &entry.resolution)
                .chain(item_resolutions.types.iter().map(|entry| &entry.resolution))
                .find_map(|resolution| def_from_resolution_named(db, resolution, name))
        })
    };
    abi_clause_source_from_lookup(db, lookup)
}

fn abi_clause_source_from_lookup<'db>(
    db: &'db dyn Db,
    mut lookup: impl FnMut(&str) -> Option<DefId<'db>>,
) -> Option<DerivedAbiClauseSource<'db>> {
    let marker = lookup("ABIDeriving")?;
    let abi_attribs = lookup("ABIAttribs")?;
    let abi_decode = lookup("ABIDecode")?;
    let word_reader = lookup("WordReader")?;
    let abi_decoder = lookup("ABIDecoder")?;
    class_named(db, marker, "ABIDeriving")?;
    class_named(db, abi_attribs, "ABIAttribs")?;
    class_named(db, abi_decode, "ABIDecode")?;
    class_named(db, word_reader, "WordReader")?;
    adt_named(db, abi_decoder, "ABIDecoder")?;
    Some(DerivedAbiClauseSource {
        marker,
        abi_attribs,
        abi_decode,
        word_reader,
        abi_decoder,
    })
}

fn def_from_resolution_named<'db>(
    db: &'db dyn Db,
    resolution: &hir_nameres::Resolution<'db>,
    name: &str,
) -> Option<DefId<'db>> {
    match resolution {
        hir_nameres::Resolution::Def { def, .. } if def.name(db).as_deref() == Some(name) => {
            Some(*def)
        }
        _ => None,
    }
}

fn local_named_def<'db>(db: &'db dyn Db, module: Module<'db>, name: &str) -> Option<DefId<'db>> {
    module.items(db).iter().find_map(|item| match item {
        Item::ClassDef(class) if class.def_id_value(db).name(db).as_deref() == Some(name) => {
            Some(class.def_id_value(db))
        }
        Item::AdtDef(adt) if adt.def_id_value(db).name(db).as_deref() == Some(name) => {
            Some(adt.def_id_value(db))
        }
        _ => None,
    })
}

fn class_named<'db>(db: &'db dyn Db, def: DefId<'db>, name: &str) -> Option<()> {
    (def.name(db).as_deref() == Some(name) && matches!(def.kind(db), hir::anchor::DefKind::Class))
        .then_some(())
}

fn adt_named<'db>(db: &'db dyn Db, def: DefId<'db>, name: &str) -> Option<()> {
    (def.name(db).as_deref() == Some(name) && matches!(def.kind(db), hir::anchor::DefKind::Adt))
        .then_some(())
}

pub(super) fn push_derived_abi_clauses<'db>(
    db: &'db dyn Db,
    clauses: &mut Vec<ProgramClause<'db>>,
    info: &AdtDeriveInfo<'db>,
    plan: &DerivedGenericPlan<'db>,
    source: DerivedAbiClauseSource<'db>,
) {
    let adt = info.adt.def_id_value(db);
    if ty_mentions_adt(db, plan.rep, adt) {
        return;
    }

    let params = info
        .adt
        .ty_param_elems(db)
        .iter()
        .enumerate()
        .map(|(index, _)| Ty::bound(db, index as u32))
        .collect::<Vec<_>>();
    let main = Ty::named(
        db,
        TyCtor::User(crate::UserTyCtor {
            def: adt,
            kind: crate::UserTyCtorKind::Adt,
        }),
        params.clone(),
    );

    clauses.push(ProgramClause {
        binder_count: info.type_vars.len() as u32,
        head: Pred::in_class(db, ClassId::User(source.abi_attribs), main, Vec::new()),
        conditions: params
            .iter()
            .map(|param| Pred::in_class(db, ClassId::User(source.abi_attribs), *param, Vec::new()))
            .collect(),
        origin: ClauseOrigin::Derived(DerivedClauseKind::AbiAttribs { adt }),
    });

    let reader = Ty::bound(db, info.type_vars.len() as u32);
    let decoder = |decoded| {
        Ty::named(
            db,
            TyCtor::User(crate::UserTyCtor {
                def: source.abi_decoder,
                kind: crate::UserTyCtorKind::Adt,
            }),
            vec![decoded, reader],
        )
    };
    let mut conditions = Vec::with_capacity(params.len() + 1);
    conditions.push(Pred::in_class(
        db,
        ClassId::User(source.word_reader),
        reader,
        Vec::new(),
    ));
    conditions.extend(params.iter().map(|param| {
        Pred::in_class(
            db,
            ClassId::User(source.abi_decode),
            decoder(*param),
            vec![*param],
        )
    }));
    clauses.push(ProgramClause {
        binder_count: info.type_vars.len() as u32 + 1,
        head: Pred::in_class(
            db,
            ClassId::User(source.abi_decode),
            decoder(main),
            vec![main],
        ),
        conditions,
        origin: ClauseOrigin::Derived(DerivedClauseKind::AbiDecode {
            adt,
            word_reader: source.word_reader,
        }),
    });
}

fn ty_mentions_adt<'db>(db: &'db dyn Db, ty: Ty<'db>, needle: DefId<'db>) -> bool {
    match ty.kind(db) {
        TyKind::Named { ctor, args } => {
            matches!(
                ctor,
                TyCtor::User(crate::UserTyCtor {
                    def,
                    kind: crate::UserTyCtorKind::Adt,
                }) if *def == needle
            ) || args.iter().any(|arg| ty_mentions_adt(db, *arg, needle))
        }
        TyKind::Function { params, ret } => {
            params
                .iter()
                .any(|param| ty_mentions_adt(db, *param, needle))
                || ty_mentions_adt(db, *ret, needle)
        }
        TyKind::Tuple(elems) => elems.iter().any(|elem| ty_mentions_adt(db, *elem, needle)),
        TyKind::Comptime(inner) => ty_mentions_adt(db, *inner, needle),
        TyKind::Error | TyKind::Unknown | TyKind::BoundVar(_) => false,
    }
}
