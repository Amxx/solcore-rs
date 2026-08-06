use super::*;

/// Returns solver plans for all valid class derivations declared in `module`.
#[salsa::tracked(returns(ref))]
pub fn derived_class_plans<'db>(
    db: &'db dyn Db,
    module: ModuleId<'db>,
) -> Vec<DerivedClassPlan<'db>> {
    let Some((scope, item_resolutions)) = scope_resolution_for_module_id(db, module) else {
        return Vec::new();
    };
    derived_class_plans_with_resolutions(db, scope.module, &item_resolutions)
}

pub(super) fn derived_class_plans_with_resolutions<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    item_resolutions: &hir_nameres::ItemResolutionFacts<'db>,
) -> Vec<DerivedClassPlan<'db>> {
    let infos = local_adt_infos(db, module);
    let mut plans = Vec::new();
    for derive in &item_resolutions.derives {
        let hir_nameres::Resolution::Def {
            def: class,
            kind: hir_nameres::DefResolutionKind::Class,
        } = derive.resolution
        else {
            continue;
        };
        if !user_class_supports_deriving(db, class) {
            continue;
        }
        let Some(info) = infos
            .iter()
            .find(|info| info.adt.def_id_value(db) == derive.adt)
        else {
            continue;
        };
        let own_param_count = info.adt.ty_param_elems(db).len();
        // A contract-local ADT in a generic contract can capture contract type
        // variables without carrying them in its nominal type head. A solver
        // clause cannot relate such an existential binder back to the goal, so
        // deriving it would be unsound. Keep these declarations out until the
        // nominal type represents captured parameters explicitly.
        if info.type_vars.len() != own_param_count {
            continue;
        }
        let params = (0..own_param_count)
            .map(|index| Ty::bound(db, index as u32))
            .collect::<Vec<_>>();
        let main = Ty::named(
            db,
            TyCtor::User(crate::UserTyCtor {
                def: derive.adt,
                kind: crate::UserTyCtorKind::Adt,
            }),
            params.clone(),
        );
        let class_id = ClassId::User(class);
        plans.push(DerivedClassPlan {
            adt: derive.adt,
            class,
            target_index: derive.index,
            binder_count: own_param_count as u32,
            head: Pred::in_class(db, class_id, main, Vec::new()),
            conditions: params
                .into_iter()
                .map(|param| Pred::in_class(db, class_id, param, Vec::new()))
                .collect(),
            empty: info.adt.ctors(db).is_empty(),
        });
    }
    plans
}

pub(crate) fn class_derivation_diagnostics<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    item_resolutions: &hir_nameres::ItemResolutionFacts<'db>,
) -> Vec<TypeckDiagnostic> {
    let infos = local_adt_infos(db, module);
    let mut diagnostics = Vec::new();
    for derive in &item_resolutions.derives {
        let hir_nameres::Resolution::Def {
            def: class,
            kind: hir_nameres::DefResolutionKind::Class,
        } = derive.resolution
        else {
            continue;
        };
        let Some(info) = infos
            .iter()
            .find(|info| info.adt.def_id_value(db) == derive.adt)
        else {
            continue;
        };
        let span = info
            .adt
            .derives(db)
            .get(derive.index as usize)
            .map(|target| LabelSpan::from_span(db, target.span(db)))
            .unwrap_or_else(|| LabelSpan::from_span(db, info.adt.name_elem(db).span(db)));
        let ty = ident_text(db, &info.adt.name_elem(db));
        let class_name = class
            .name(db)
            .unwrap_or_else(|| display_class_source(db, ClassId::User(class)));
        if user_class_extra_arg_count(db, class).is_some_and(|arity| arity != 0) {
            diagnostics.push(TypeckDiagnostic::InvalidDerive {
                span,
                ty,
                class: class_name,
                reason: "only single-parameter classes can be derived".to_owned(),
            });
            continue;
        }
        if info.type_vars.len() != info.adt.ty_param_elems(db).len() {
            diagnostics.push(TypeckDiagnostic::InvalidDerive {
                span,
                ty,
                class: class_name,
                reason: "a contract-local data type cannot capture generic contract parameters"
                    .to_owned(),
            });
        }
    }
    diagnostics
}

pub(super) fn derived_class_target_span<'db>(
    db: &'db dyn Db,
    module: Module<'db>,
    adt: DefId<'db>,
    target_index: u32,
) -> Option<LabelSpan> {
    let info = local_adt_infos(db, module)
        .into_iter()
        .find(|info| info.adt.def_id_value(db) == adt)?;
    Some(
        info.adt
            .derives(db)
            .get(target_index as usize)
            .map(|target| LabelSpan::from_span(db, target.span(db)))
            .unwrap_or_else(|| LabelSpan::from_span(db, info.adt.name_elem(db).span(db))),
    )
}

pub(super) fn user_class_supports_deriving<'db>(db: &'db dyn Db, class: DefId<'db>) -> bool {
    user_class_extra_arg_count(db, class) == Some(0)
}

fn user_class_extra_arg_count<'db>(db: &'db dyn Db, class: DefId<'db>) -> Option<usize> {
    let module = parse_file_to_hir(db, class.file(db)).module(db);
    module.items(db).iter().find_map(|item| {
        let Item::ClassDef(def) = item else {
            return None;
        };
        (def.def_id_value(db) == class).then(|| def.head(db).kind(db).args.atom().len())
    })
}
