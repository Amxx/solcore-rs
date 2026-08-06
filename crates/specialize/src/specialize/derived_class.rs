use super::*;

#[derive(Debug, Clone)]
struct GenericInstance<'db> {
    rep: Ty<'db>,
    evidence: Evidence<'db>,
}

impl<'db> Driver<'db> {
    pub(super) fn specialize_derived_class(
        &mut self,
        key: DerivedClassKey<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<String> {
        if let Some(name) = self.derived_classes.get(&key) {
            return Some(name.clone());
        }
        if !self.ensure_specialization_type_size(&[key.main, key.target_ty], Some(span)) {
            return None;
        }
        let class_name = key
            .class
            .name(self.db)
            .unwrap_or_else(|| "Class".to_owned());
        let mut base_name = format!(
            "Derived_{class_name}_{}_{}_{}_{}",
            key.method,
            def_hash_suffix(self.db, key.class),
            def_hash_suffix(self.db, key.adt),
            key.target_index,
        );
        if !key.sub_evidence.is_empty() {
            base_name.push('_');
            base_name.push_str(&evidence_hash_suffix(self.db, &key.sub_evidence));
        }
        let name = specialize_name(self.db, &base_name, &[key.target_ty]);
        self.derived_classes.insert(key.clone(), name.clone());
        self.derived_class_order.push(key.clone());
        let Some(fun) = self.build_derived_class_function(&key, &name, span, depth) else {
            self.diagnostics.push(SpecializeDiagnostic {
                kind: SpecializeDiagnosticKind::UnsupportedEvidence {
                    context: format!("cannot generate {class_name}.{}", key.method),
                },
                span: Some(span),
            });
            self.derived_classes.remove(&key);
            self.derived_class_order
                .retain(|candidate| candidate != &key);
            return None;
        };
        self.derived_class_funs.insert(key, fun);
        Some(name)
    }

    fn build_derived_class_function(
        &mut self,
        key: &DerivedClassKey<'db>,
        name: &str,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoFunction<'db>> {
        let adt_info = self.adts.get(&key.adt)?.clone();
        let class_info = self.classes.get(&key.class)?.clone();
        let method = class_info
            .class
            .methods(self.db)
            .iter()
            .find(|candidate| ident_text(self.db, &candidate.name) == key.method)?;
        let resolution = self
            .module_resolutions
            .get(&class_info.module.def_id_value(self.db))?;
        let type_vars = hir_ty::class_method_type_vars(self.db, class_info.class, method);
        let lowerer = TypeLowering::from_item_resolutions(
            self.db,
            &resolution.item_resolutions,
            BinderEnv::from_type_vars(&type_vars),
        );
        let mut normalizer =
            AliasNormalizer::new(self.db, class_info.module, &resolution.item_resolutions);
        let scheme =
            normalizer.normalize_scheme(lowerer.lower_class_method(class_info.class, method));
        let method_ty = scheme.body(self.db).ty(self.db);
        let self_ty = scheme
            .body(self.db)
            .preds(self.db)
            .iter()
            .find_map(|pred| match pred.kind(self.db) {
                PredKind::InClass {
                    class: ClassId::User(class),
                    main,
                    args,
                } if *class == key.class && args.is_empty() => Some(*main),
                _ => None,
            })?;
        let mut method_subst = TySubst::default();
        if !method_subst.match_ty(self.db, method_ty, key.target_ty)
            || method_subst.apply_ty(self.db, self_ty) != key.main
        {
            return None;
        }
        let TyKind::Function {
            params: method_params,
            ret: method_ret,
        } = method_ty.kind(self.db)
        else {
            return None;
        };
        let TyKind::Function {
            params: target_params,
            ret: target_ret,
        } = strip_comptime_ty(self.db, key.target_ty).kind(self.db)
        else {
            return None;
        };
        if method.params.atom().len() != target_params.len()
            || method_params.len() != target_params.len()
        {
            return None;
        }
        if !target_params
            .iter()
            .chain(std::iter::once(target_ret))
            .all(|ty| ty_is_closed(self.db, *ty))
        {
            return None;
        }
        let bare_self = strip_comptime_ty(self.db, self_ty);
        if method_params.iter().any(|ty| {
            let ty = strip_comptime_ty(self.db, *ty);
            ty != bare_self && ty_contains(self.db, ty, bare_self)
        }) {
            return None;
        }
        let bare_ret = strip_comptime_ty(self.db, *method_ret);
        if bare_ret != bare_self && ty_contains(self.db, bare_ret, bare_self) {
            return None;
        }

        let params = method
            .params
            .atom()
            .iter()
            .zip(target_params)
            .map(|(source, ty)| MonoParam {
                name: param_name(self.db, source).unwrap_or("_").to_owned(),
                mode: ParamMode::from_bool(param_comptime(source) || ty_is_comptime(self.db, *ty)),
                ty: MonoTy::new_unchecked(*ty),
                span: source.span(self.db),
            })
            .collect::<Vec<_>>();

        let body_expr = if adt_info.adt.ctors(self.db).is_empty() {
            self.derived_class_absurd_expr(*target_ret, span, depth)?
        } else {
            let generic = self.resolve_generic_instance(&adt_info, key.main)?;
            self.derived_class_delegated_expr(
                key,
                method_params,
                *method_ret,
                self_ty,
                &params,
                *target_ret,
                &generic,
                span,
                depth,
            )?
        };

        Some(MonoFunction {
            origin: MonoFunctionOrigin::DerivedClass {
                adt: key.adt,
                class: key.class,
                target_index: key.target_index,
                method: key.method.clone(),
            },
            source: None,
            shadowed_top_level: None,
            name: name.to_owned(),
            span,
            params,
            ret: MonoTy::new_unchecked(*target_ret),
            comptime_obligations: Vec::new(),
            body: vec![MonoStmt {
                span,
                kind: MonoStmtKind::Return(Some(body_expr)),
            }],
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn derived_class_delegated_expr(
        &mut self,
        key: &DerivedClassKey<'db>,
        method_params: &[Ty<'db>],
        method_ret: Ty<'db>,
        self_ty: Ty<'db>,
        params: &[MonoParam<'db>],
        target_ret: Ty<'db>,
        generic: &GenericInstance<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoExpr<'db>> {
        let rep = generic.rep;
        let mut args = Vec::with_capacity(params.len());
        let mut delegated_params = Vec::with_capacity(params.len());
        for ((source_ty, param), target_ty) in method_params
            .iter()
            .zip(params)
            .zip(params.iter().map(|param| param.ty.ty()))
        {
            let var = MonoExpr {
                span: param.span,
                ty: param.ty,
                kind: MonoExprKind::Var(MonoId {
                    name: param.name.clone(),
                    ty: param.ty,
                    span: param.span,
                }),
            };
            if strip_comptime_ty(self.db, *source_ty) == strip_comptime_ty(self.db, self_ty) {
                let from_ty = Ty::function(self.db, vec![key.main], rep);
                let from = self.resolve_class_method_call(
                    "from",
                    generic.evidence.clone(),
                    from_ty,
                    span,
                    depth + 1,
                )?;
                args.push(MonoExpr {
                    span: param.span,
                    ty: MonoTy::new_unchecked(rep),
                    kind: MonoExprKind::Call {
                        callee: MonoId {
                            name: from,
                            ty: MonoTy::new_unchecked(from_ty),
                            span: param.span,
                        },
                        args: vec![var],
                        origin: MonoCallOrigin::ByName,
                    },
                });
                delegated_params.push(rep);
            } else {
                args.push(var);
                delegated_params.push(target_ty);
            }
        }
        let returns_self =
            strip_comptime_ty(self.db, method_ret) == strip_comptime_ty(self.db, self_ty);
        let delegated_ret = if returns_self { rep } else { target_ret };
        let delegated_ty = Ty::function(self.db, delegated_params, delegated_ret);
        let evidence = self.solve_class_method_pred_with_givens(
            key.class,
            &key.method,
            delegated_ty,
            key,
            Some(span),
        )?;
        let delegated_name =
            self.resolve_class_method_call(&key.method, evidence, delegated_ty, span, depth + 1)?;
        let delegated = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(delegated_ret),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name: delegated_name,
                    ty: MonoTy::new_unchecked(delegated_ty),
                    span,
                },
                args,
                origin: MonoCallOrigin::ByName,
            },
        };
        if !returns_self {
            return Some(delegated);
        }
        let to_ty = Ty::function(self.db, vec![rep], key.main);
        let to =
            self.resolve_class_method_call("to", generic.evidence.clone(), to_ty, span, depth + 1)?;
        Some(MonoExpr {
            span,
            ty: MonoTy::new_unchecked(key.main),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name: to,
                    ty: MonoTy::new_unchecked(to_ty),
                    span,
                },
                args: vec![delegated],
                origin: MonoCallOrigin::ByName,
            },
        })
    }

    fn solve_class_method_pred_with_givens(
        &mut self,
        class: DefId<'db>,
        method: &str,
        callee_ty: Ty<'db>,
        key: &DerivedClassKey<'db>,
        span: Option<Span<'db>>,
    ) -> Option<Evidence<'db>> {
        let TyKind::Named { args, .. } = key.main.kind(self.db) else {
            return None;
        };
        if args.len() != key.sub_evidence.len() {
            return None;
        }
        let givens = args
            .iter()
            .map(|arg| Pred::in_class(self.db, ClassId::User(class), *arg, Vec::new()))
            .collect::<Vec<_>>();
        let method_info = self.classes.get(&class)?.clone();
        let method_sig = method_info
            .class
            .methods(self.db)
            .iter()
            .find(|candidate| ident_text(self.db, &candidate.name) == method)?;
        let resolution = self
            .module_resolutions
            .get(&method_info.module.def_id_value(self.db))?;
        let method_vars = hir_ty::class_method_type_vars(self.db, method_info.class, method_sig);
        let lowerer = TypeLowering::from_item_resolutions(
            self.db,
            &resolution.item_resolutions,
            BinderEnv::from_type_vars(&method_vars),
        );
        let mut normalizer =
            AliasNormalizer::new(self.db, method_info.module, &resolution.item_resolutions);
        let scheme =
            normalizer.normalize_scheme(lowerer.lower_class_method(method_info.class, method_sig));
        let mut subst = TySubst::default();
        if !subst.match_ty(self.db, scheme.body(self.db).ty(self.db), callee_ty) {
            return None;
        }
        let goal = scheme
            .body(self.db)
            .preds(self.db)
            .iter()
            .map(|pred| subst.apply_pred(self.db, *pred))
            .find(|pred| matches!(pred.kind(self.db), PredKind::InClass { class: ClassId::User(def), .. } if *def == class))?;
        let adt_module = self.adts.get(&key.adt)?.module;
        let base = self.try_module_trait_env(adt_module).or_else(|| {
            self.push_missing_module_trait_env(span);
            None
        })?;
        let env = trait_env_with_givens(self.db, base, givens.clone());
        let Solution::Unique { evidence, .. } = solve(self.db, env, canonical_goal(self.db, goal))
        else {
            return None;
        };
        let bindings = givens
            .into_iter()
            .zip(key.sub_evidence.iter().cloned())
            .collect::<Vec<_>>();
        Some(replay_evidence_bindings(evidence, &bindings))
    }

    fn resolve_generic_instance(
        &mut self,
        adt_info: &AdtInfo<'db>,
        main: Ty<'db>,
    ) -> Option<GenericInstance<'db>> {
        let generic = self.generic_class_for_module(adt_info.module)?;
        let rep_var = Ty::bound(self.db, 0);
        let goal = Pred::in_class(self.db, ClassId::User(generic), main, vec![rep_var]);
        let env = self.try_module_trait_env(adt_info.module)?;
        let Solution::Unique {
            subst,
            mut evidence,
        } = solve(
            self.db,
            env,
            canonical_goal_with_allowed(self.db, goal, vec![0]),
        )
        else {
            return None;
        };
        let mut solution_subst = TySubst::default();
        for (index, ty) in subst.values {
            solution_subst.insert_if_consistent(index, ty);
        }
        let rep = solution_subst.apply_ty(self.db, rep_var);
        if !ty_is_closed(self.db, rep) {
            return None;
        }
        evidence = solution_subst.apply_evidence(self.db, evidence);
        Some(GenericInstance { rep, evidence })
    }

    fn generic_class_for_module(&self, module: Module<'db>) -> Option<DefId<'db>> {
        let imported =
            module_id_for_source_file(self.db, module.def_id_value(self.db).file(self.db))
                .and_then(|module_id| {
                    let surface = nameres::module_import_surface(self.db, module_id);
                    surface
                        .types
                        .get("Generic")
                        .and_then(|resolution| generic_class_resolution(self.db, resolution))
                        .or_else(|| {
                            surface
                                .item_scope
                                .as_ref()
                                .and_then(|scope| scope.types.get("Generic"))
                                .and_then(|entry| {
                                    generic_class_resolution(self.db, &entry.resolution)
                                })
                        })
                });
        imported.or_else(|| {
            module.items(self.db).iter().find_map(|item| {
                let Item::ClassDef(class) = item else {
                    return None;
                };
                (class.def_id_value(self.db).name(self.db).as_deref() == Some("Generic"))
                    .then_some(class.def_id_value(self.db))
            })
        })
    }

    fn derived_class_absurd_expr(
        &mut self,
        ret: Ty<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoExpr<'db>> {
        let def = self.functions.iter().find_map(|(def, info)| {
            (ident_text(self.db, &info.function.sig(self.db).name) == "absurd"
                && module_is_std_root(self.db, info.module))
            .then_some(*def)
        })?;
        let callee_ty = Ty::function(self.db, Vec::new(), ret);
        let name = self.specialize_source_function(def, callee_ty, span, depth + 1)?;
        Some(MonoExpr {
            span,
            ty: MonoTy::new_unchecked(ret),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name,
                    ty: MonoTy::new_unchecked(callee_ty),
                    span,
                },
                args: Vec::new(),
                origin: MonoCallOrigin::Source(def),
            },
        })
    }

    fn specialize_source_function(
        &mut self,
        def: DefId<'db>,
        callee_ty: Ty<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<String> {
        if !self.ensure_specialization_type_size(&[callee_ty], Some(span)) {
            return None;
        }
        let info = self.functions.get(&def)?.clone();
        let base = self.source_base_name(&info);
        let lowered = self.try_lower_normalized_function(&info)?;
        let mut subst = TySubst::default();
        if !subst.match_ty(self.db, lowered.scheme.body(self.db).ty(self.db), callee_ty) {
            return None;
        }
        let givens = self.function_givens(&info, &lowered);
        self.resolve_mptc_from_preds(info.module, &givens, &mut subst);
        let args = subst.specialization_args();
        let key = SpecKey {
            def,
            ty: callee_ty,
            base_name: specialize_name(self.db, &base, &args),
            origin: MonoFunctionOrigin::Source,
            evidence_bindings: Vec::new(),
        };
        Some(self.enqueue(key, depth))
    }
}

fn generic_class_resolution<'db>(
    db: &'db dyn Db,
    resolution: &hir_nameres::Resolution<'db>,
) -> Option<DefId<'db>> {
    match resolution {
        hir_nameres::Resolution::Def {
            def,
            kind: hir_nameres::DefResolutionKind::Class,
        } if def.name(db).as_deref() == Some("Generic") => Some(*def),
        _ => None,
    }
}

fn ty_contains<'db>(db: &'db dyn Db, ty: Ty<'db>, needle: Ty<'db>) -> bool {
    if ty == needle {
        return true;
    }
    match ty.kind(db) {
        TyKind::Named { args, .. } | TyKind::Tuple(args) => {
            args.iter().any(|arg| ty_contains(db, *arg, needle))
        }
        TyKind::Function { params, ret } => {
            params.iter().any(|param| ty_contains(db, *param, needle))
                || ty_contains(db, *ret, needle)
        }
        TyKind::Comptime(inner) => ty_contains(db, *inner, needle),
        TyKind::BoundVar(_) | TyKind::Error | TyKind::Unknown => false,
    }
}

fn module_is_std_root<'db>(db: &'db dyn Db, module: Module<'db>) -> bool {
    let Some(module_id) = module_id_for_source_file(db, module.def_id_value(db).file(db)) else {
        return false;
    };
    *module_id.library(db) == LibraryId::Std && module_id.logical_path(db).as_slice() == ["std"]
}
