use super::*;

impl<'db> Driver<'db> {
    pub(super) fn specialize_derived_storage(
        &mut self,
        key: DerivedStorageKey<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<String> {
        if let Some(name) = self.derived_storages.get(&key) {
            return Some(name.clone());
        }
        if !self.ensure_specialization_type_size(&[key.target_ty], Some(span)) {
            return None;
        }

        let family = match key.family {
            DerivedStorageFamily::Size => "StorageSize",
            DerivedStorageFamily::CanStore => "CanStore",
        };
        let PredKind::InClass {
            class: ClassId::User(class),
            ..
        } = key.pred.kind(self.db)
        else {
            return None;
        };
        let mut base = format!(
            "Derived_{family}_{}_{}_{}",
            key.method,
            def_hash_suffix(self.db, key.adt),
            def_hash_suffix(self.db, *class),
        );
        if !key.sub_evidence.is_empty() {
            base.push('_');
            base.push_str(&evidence_hash_suffix(self.db, &key.sub_evidence));
        }
        let name = specialize_name(self.db, &base, &[key.target_ty]);
        self.derived_storages.insert(key.clone(), name.clone());
        self.derived_storage_order.push(key.clone());

        let fun = match key.family {
            DerivedStorageFamily::Size => {
                self.build_derived_storage_size_function(&key, &name, span, depth)
            }
            DerivedStorageFamily::CanStore => {
                self.build_derived_can_store_function(&key, &name, span, depth)
            }
        };
        let Some(fun) = fun else {
            self.diagnostics.push(SpecializeDiagnostic {
                kind: SpecializeDiagnosticKind::UnsupportedEvidence {
                    context: format!("cannot generate {family}.{}", key.method),
                },
                span: Some(span),
            });
            self.derived_storages.remove(&key);
            self.derived_storage_order
                .retain(|candidate| candidate != &key);
            return None;
        };
        self.derived_storage_funs.insert(key, fun);
        Some(name)
    }

    fn build_derived_storage_size_function(
        &mut self,
        key: &DerivedStorageKey<'db>,
        name: &str,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoFunction<'db>> {
        let PredKind::InClass {
            class: ClassId::User(class),
            main,
            args,
        } = key.pred.kind(self.db)
        else {
            return None;
        };
        if !args.is_empty() || key.method != "size" || key.storage_size.is_some() {
            return None;
        }
        let TyKind::Named {
            ctor:
                TyCtor::User(UserTyCtor {
                    def: main_adt,
                    kind: UserTyCtorKind::Adt,
                }),
            args: main_args,
        } = main.kind(self.db)
        else {
            return None;
        };
        if *main_adt != key.adt {
            return None;
        }

        let adt_info = self.adts.get(&key.adt)?.clone();
        let generic = self.resolve_generic_instance(&adt_info, *main)?;
        let TyKind::Function { params, ret } =
            strip_comptime_ty(self.db, key.target_ty).kind(self.db)
        else {
            return None;
        };
        let [proxy_main] = params.as_slice() else {
            return None;
        };
        let TyKind::Named {
            ctor: proxy_ctor,
            args: proxy_args,
        } = strip_comptime_ty(self.db, *proxy_main).kind(self.db)
        else {
            return None;
        };
        if proxy_args.as_slice() != [*main] {
            return None;
        }

        let proxy_rep = Ty::named(self.db, *proxy_ctor, vec![generic.rep]);
        let delegated_pred =
            Pred::in_class(self.db, ClassId::User(*class), generic.rep, Vec::new());
        let delegated_ty = Ty::function(self.db, vec![proxy_rep], *ret);
        let givens = main_args
            .iter()
            .map(|arg| Pred::in_class(self.db, ClassId::User(*class), *arg, Vec::new()))
            .collect();
        let evidence =
            self.solve_derived_storage_pred_with_givens(key, delegated_pred, givens, Some(span))?;
        let delegated =
            self.resolve_class_method_call("size", evidence, delegated_ty, span, depth + 1)?;

        let param = MonoParam {
            name: "_x".to_owned(),
            mode: ParamMode::Runtime,
            ty: MonoTy::new_unchecked(*proxy_main),
            span,
        };
        let proxy = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(proxy_rep),
            kind: MonoExprKind::Proxy(MonoTy::new_unchecked(generic.rep)),
        };
        let call = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(*ret),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name: delegated,
                    ty: MonoTy::new_unchecked(delegated_ty),
                    span,
                },
                args: vec![proxy],
                origin: MonoCallOrigin::ByName,
            },
        };
        Some(MonoFunction {
            origin: MonoFunctionOrigin::DerivedGeneric {
                adt: key.adt,
                method: "StorageSize.size".to_owned(),
            },
            source: None,
            shadowed_top_level: None,
            name: name.to_owned(),
            span,
            params: vec![param],
            ret: MonoTy::new_unchecked(*ret),
            comptime_obligations: Vec::new(),
            body: vec![MonoStmt {
                span,
                kind: MonoStmtKind::Return(Some(call)),
            }],
        })
    }

    fn build_derived_can_store_function(
        &mut self,
        key: &DerivedStorageKey<'db>,
        name: &str,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoFunction<'db>> {
        let PredKind::InClass {
            class: ClassId::User(class),
            main: storage_main,
            args,
        } = key.pred.kind(self.db)
        else {
            return None;
        };
        let [main] = args.as_slice() else {
            return None;
        };
        let storage_size = key.storage_size?;
        let TyKind::Named {
            ctor: storage_ctor,
            args: storage_args,
        } = storage_main.kind(self.db)
        else {
            return None;
        };
        if storage_args.as_slice() != [*main] {
            return None;
        }
        let TyCtor::User(storage_user) = storage_ctor else {
            return None;
        };
        let TyKind::Named {
            ctor:
                TyCtor::User(UserTyCtor {
                    def: main_adt,
                    kind: UserTyCtorKind::Adt,
                }),
            args: main_args,
        } = main.kind(self.db)
        else {
            return None;
        };
        if *main_adt != key.adt {
            return None;
        }

        let adt_info = self.adts.get(&key.adt)?.clone();
        let generic = self.resolve_generic_instance(&adt_info, *main)?;
        let rep_storage = Ty::named(self.db, *storage_ctor, vec![generic.rep]);
        let delegated_pred = Pred::in_class(
            self.db,
            ClassId::User(*class),
            rep_storage,
            vec![generic.rep],
        );
        let mut givens = Vec::with_capacity(main_args.len() * 2);
        givens.extend(main_args.iter().map(|arg| {
            let param_storage = Ty::named(self.db, *storage_ctor, vec![*arg]);
            Pred::in_class(self.db, ClassId::User(*class), param_storage, vec![*arg])
        }));
        givens.extend(
            main_args
                .iter()
                .map(|arg| Pred::in_class(self.db, ClassId::User(storage_size), *arg, Vec::new())),
        );
        let evidence =
            self.solve_derived_storage_pred_with_givens(key, delegated_pred, givens, Some(span))?;

        let TyKind::Function { params, ret } =
            strip_comptime_ty(self.db, key.target_ty).kind(self.db)
        else {
            return None;
        };
        let (delegated_params, result_from_rep) = match key.method.as_str() {
            "store" => {
                let [slot, value] = params.as_slice() else {
                    return None;
                };
                if *slot != *storage_main || *value != *main {
                    return None;
                }
                (vec![rep_storage, generic.rep], false)
            }
            "load" => {
                let [slot] = params.as_slice() else {
                    return None;
                };
                if *slot != *storage_main || *ret != *main {
                    return None;
                }
                (vec![rep_storage], true)
            }
            _ => return None,
        };
        let delegated_ret = if result_from_rep { generic.rep } else { *ret };
        let delegated_ty = Ty::function(self.db, delegated_params, delegated_ret);
        let delegated =
            self.resolve_class_method_call(&key.method, evidence, delegated_ty, span, depth + 1)?;

        let storage_info = self.adts.get(&storage_user.def)?.clone();
        let storage_ctor_name = storage_info.adt.ctors(self.db).first().map(|ctor| {
            format!(
                "{}_{}",
                storage_user
                    .def
                    .name(self.db)
                    .unwrap_or_else(|| "storage".to_owned()),
                ident_text(self.db, &ctor.name),
            )
        })?;
        let slot_param = MonoParam {
            name: "_r".to_owned(),
            mode: ParamMode::Runtime,
            ty: MonoTy::new_unchecked(*storage_main),
            span,
        };
        let slot_word = MonoId {
            name: "_slot".to_owned(),
            ty: MonoTy::new_unchecked(Ty::word(self.db)),
            span,
        };
        let rep_slot = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(rep_storage),
            kind: MonoExprKind::Con {
                ctor: MonoId {
                    name: storage_ctor_name.clone(),
                    ty: MonoTy::new_unchecked(rep_storage),
                    span,
                },
                args: vec![MonoExpr {
                    span,
                    ty: slot_word.ty,
                    kind: MonoExprKind::Var(slot_word.clone()),
                }],
            },
        };

        let (params, result) = if result_from_rep {
            let loaded_rep = MonoExpr {
                span,
                ty: MonoTy::new_unchecked(generic.rep),
                kind: MonoExprKind::Call {
                    callee: MonoId {
                        name: delegated,
                        ty: MonoTy::new_unchecked(delegated_ty),
                        span,
                    },
                    args: vec![rep_slot],
                    origin: MonoCallOrigin::ByName,
                },
            };
            let to_ty = Ty::function(self.db, vec![generic.rep], *main);
            let to =
                self.resolve_class_method_call("to", generic.evidence, to_ty, span, depth + 1)?;
            (
                vec![slot_param.clone()],
                MonoExpr {
                    span,
                    ty: MonoTy::new_unchecked(*main),
                    kind: MonoExprKind::Call {
                        callee: MonoId {
                            name: to,
                            ty: MonoTy::new_unchecked(to_ty),
                            span,
                        },
                        args: vec![loaded_rep],
                        origin: MonoCallOrigin::ByName,
                    },
                },
            )
        } else {
            let value_param = MonoParam {
                name: "_v".to_owned(),
                mode: ParamMode::Runtime,
                ty: MonoTy::new_unchecked(*main),
                span,
            };
            let value = MonoExpr {
                span,
                ty: value_param.ty,
                kind: MonoExprKind::Var(MonoId {
                    name: value_param.name.clone(),
                    ty: value_param.ty,
                    span,
                }),
            };
            let from_ty = Ty::function(self.db, vec![*main], generic.rep);
            let from =
                self.resolve_class_method_call("from", generic.evidence, from_ty, span, depth + 1)?;
            let rep_value = MonoExpr {
                span,
                ty: MonoTy::new_unchecked(generic.rep),
                kind: MonoExprKind::Call {
                    callee: MonoId {
                        name: from,
                        ty: MonoTy::new_unchecked(from_ty),
                        span,
                    },
                    args: vec![value],
                    origin: MonoCallOrigin::ByName,
                },
            };
            (
                vec![slot_param.clone(), value_param],
                MonoExpr {
                    span,
                    ty: MonoTy::new_unchecked(*ret),
                    kind: MonoExprKind::Call {
                        callee: MonoId {
                            name: delegated,
                            ty: MonoTy::new_unchecked(delegated_ty),
                            span,
                        },
                        args: vec![rep_slot, rep_value],
                        origin: MonoCallOrigin::ByName,
                    },
                },
            )
        };

        let slot_var = MonoExpr {
            span,
            ty: slot_param.ty,
            kind: MonoExprKind::Var(MonoId {
                name: slot_param.name.clone(),
                ty: slot_param.ty,
                span,
            }),
        };
        let slot_pat = MonoPat {
            span,
            ty: MonoTy::new_unchecked(*storage_main),
            kind: MonoPatKind::Con {
                ctor: MonoId {
                    name: storage_ctor_name,
                    ty: MonoTy::new_unchecked(*storage_main),
                    span,
                },
                args: vec![MonoPat {
                    span,
                    ty: slot_word.ty,
                    kind: MonoPatKind::Var(slot_word),
                }],
            },
        };
        Some(MonoFunction {
            origin: MonoFunctionOrigin::DerivedGeneric {
                adt: key.adt,
                method: format!("CanStore.{}", key.method),
            },
            source: None,
            shadowed_top_level: None,
            name: name.to_owned(),
            span,
            params,
            ret: MonoTy::new_unchecked(*ret),
            comptime_obligations: Vec::new(),
            body: vec![MonoStmt {
                span,
                kind: MonoStmtKind::Match {
                    scrutinees: vec![slot_var],
                    arms: vec![MonoArm {
                        span,
                        pats: vec![slot_pat],
                        body: vec![MonoStmt {
                            span,
                            kind: MonoStmtKind::Return(Some(result)),
                        }],
                    }],
                },
            }],
        })
    }

    fn solve_derived_storage_pred_with_givens(
        &mut self,
        key: &DerivedStorageKey<'db>,
        pred: Pred<'db>,
        givens: Vec<Pred<'db>>,
        span: Option<Span<'db>>,
    ) -> Option<Evidence<'db>> {
        if givens.len() != key.sub_evidence.len() {
            return None;
        }
        let adt_module = self.adts.get(&key.adt)?.module;
        let base = self.try_module_trait_env(adt_module).or_else(|| {
            self.push_missing_module_trait_env(span);
            None
        })?;
        let env = trait_env_with_givens(self.db, base, givens.clone());
        let Solution::Unique { evidence, .. } = solve(self.db, env, canonical_goal(self.db, pred))
        else {
            return None;
        };
        let bindings = givens
            .into_iter()
            .zip(key.sub_evidence.iter().cloned())
            .collect::<Vec<_>>();
        Some(replay_evidence_bindings(evidence, &bindings))
    }
}
