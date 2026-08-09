use super::*;

impl<'db> Driver<'db> {
    pub(super) fn specialize_derived_abi(
        &mut self,
        key: DerivedAbiKey<'db>,
        span: Span<'db>,
        depth: usize,
    ) -> Option<String> {
        if let Some(name) = self.derived_abis.get(&key) {
            return Some(name.clone());
        }
        if !self.ensure_specialization_type_size(&[key.target_ty], Some(span)) {
            return None;
        }

        let family = match key.family {
            DerivedAbiFamily::Attribs => "ABIAttribs",
            DerivedAbiFamily::Decode => "ABIDecode",
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
        self.derived_abis.insert(key.clone(), name.clone());
        self.derived_abi_order.push(key.clone());

        let fun = match key.family {
            DerivedAbiFamily::Attribs => {
                self.build_derived_abi_attribs_function(&key, &name, span, depth)
            }
            DerivedAbiFamily::Decode => {
                self.build_derived_abi_decode_function(&key, &name, span, depth)
            }
        };
        let Some(fun) = fun else {
            self.diagnostics.push(SpecializeDiagnostic {
                kind: SpecializeDiagnosticKind::UnsupportedEvidence {
                    context: format!("cannot generate {family}.{}", key.method),
                },
                span: Some(span),
            });
            self.derived_abis.remove(&key);
            self.derived_abi_order.retain(|candidate| candidate != &key);
            return None;
        };
        self.derived_abi_funs.insert(key, fun);
        Some(name)
    }

    fn build_derived_abi_attribs_function(
        &mut self,
        key: &DerivedAbiKey<'db>,
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
        if !args.is_empty() || !matches!(key.method.as_str(), "headSize" | "isStatic") {
            return None;
        }
        if key.word_reader.is_some() {
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
            self.solve_derived_abi_pred_with_givens(key, delegated_pred, givens, Some(span))?;
        let delegated =
            self.resolve_class_method_call(&key.method, evidence, delegated_ty, span, depth + 1)?;

        let param = MonoParam {
            name: "_ty".to_owned(),
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
                method: format!("ABIAttribs.{}", key.method),
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

    fn build_derived_abi_decode_function(
        &mut self,
        key: &DerivedAbiKey<'db>,
        name: &str,
        span: Span<'db>,
        depth: usize,
    ) -> Option<MonoFunction<'db>> {
        let PredKind::InClass {
            class: ClassId::User(class),
            main: decoder,
            args,
        } = key.pred.kind(self.db)
        else {
            return None;
        };
        let [decoded] = args.as_slice() else {
            return None;
        };
        if key.method != "decode" {
            return None;
        }
        let TyKind::Named {
            ctor: decoder_ctor,
            args: decoder_args,
        } = decoder.kind(self.db)
        else {
            return None;
        };
        let [decoder_decoded, reader] = decoder_args.as_slice() else {
            return None;
        };
        if decoder_decoded != decoded {
            return None;
        }
        let TyCtor::User(decoder_user) = decoder_ctor else {
            return None;
        };
        let TyKind::Named {
            ctor:
                TyCtor::User(UserTyCtor {
                    def: decoded_adt,
                    kind: UserTyCtorKind::Adt,
                }),
            args: decoded_args,
        } = decoded.kind(self.db)
        else {
            return None;
        };
        if *decoded_adt != key.adt {
            return None;
        }
        let word_reader = key.word_reader?;
        let decoder_info = self.adts.get(&decoder_user.def)?.clone();
        let decoder_ctor_name = decoder_info.adt.ctors(self.db).first().map(|ctor| {
            format!(
                "{}_{}",
                decoder_user
                    .def
                    .name(self.db)
                    .unwrap_or_else(|| "ABIDecoder".to_owned()),
                ident_text(self.db, &ctor.name),
            )
        })?;

        let adt_info = self.adts.get(&key.adt)?.clone();
        let generic = self.resolve_generic_instance(&adt_info, *decoded)?;
        let rep_decoder = Ty::named(self.db, *decoder_ctor, vec![generic.rep, *reader]);
        let delegated_pred = Pred::in_class(
            self.db,
            ClassId::User(*class),
            rep_decoder,
            vec![generic.rep],
        );
        let TyKind::Function { params, ret } =
            strip_comptime_ty(self.db, key.target_ty).kind(self.db)
        else {
            return None;
        };
        let [decoder_param, head_offset] = params.as_slice() else {
            return None;
        };
        if *decoder_param != *decoder || *ret != *decoded {
            return None;
        }

        let delegated_ty = Ty::function(self.db, vec![rep_decoder, *head_offset], generic.rep);
        let mut givens = Vec::with_capacity(decoded_args.len() + 1);
        givens.push(Pred::in_class(
            self.db,
            ClassId::User(word_reader),
            *reader,
            Vec::new(),
        ));
        givens.extend(decoded_args.iter().map(|arg| {
            let param_decoder = Ty::named(self.db, *decoder_ctor, vec![*arg, *reader]);
            Pred::in_class(self.db, ClassId::User(*class), param_decoder, vec![*arg])
        }));
        let evidence =
            self.solve_derived_abi_pred_with_givens(key, delegated_pred, givens, Some(span))?;
        let delegated =
            self.resolve_class_method_call("decode", evidence, delegated_ty, span, depth + 1)?;
        let to_ty = Ty::function(self.db, vec![generic.rep], *decoded);
        let to = self.resolve_class_method_call("to", generic.evidence, to_ty, span, depth + 1)?;

        let ptr_param = MonoParam {
            name: "_ptr".to_owned(),
            mode: ParamMode::Runtime,
            ty: MonoTy::new_unchecked(*decoder_param),
            span,
        };
        let head_param = MonoParam {
            name: "_headOffset".to_owned(),
            mode: ParamMode::Runtime,
            ty: MonoTy::new_unchecked(*head_offset),
            span,
        };
        let rdr_id = MonoId {
            name: "_rdr".to_owned(),
            ty: MonoTy::new_unchecked(*reader),
            span,
        };
        let rep_ptr = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(rep_decoder),
            kind: MonoExprKind::Con {
                ctor: MonoId {
                    name: decoder_ctor_name.clone(),
                    ty: MonoTy::new_unchecked(rep_decoder),
                    span,
                },
                args: vec![MonoExpr {
                    span,
                    ty: MonoTy::new_unchecked(*reader),
                    kind: MonoExprKind::Var(rdr_id.clone()),
                }],
            },
        };
        let head_arg = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(*head_offset),
            kind: MonoExprKind::Var(MonoId {
                name: head_param.name.clone(),
                ty: head_param.ty,
                span,
            }),
        };
        let decoded_rep = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(generic.rep),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name: delegated,
                    ty: MonoTy::new_unchecked(delegated_ty),
                    span,
                },
                args: vec![rep_ptr, head_arg],
                origin: MonoCallOrigin::ByName,
            },
        };
        let result = MonoExpr {
            span,
            ty: MonoTy::new_unchecked(*decoded),
            kind: MonoExprKind::Call {
                callee: MonoId {
                    name: to,
                    ty: MonoTy::new_unchecked(to_ty),
                    span,
                },
                args: vec![decoded_rep],
                origin: MonoCallOrigin::ByName,
            },
        };
        let ptr = MonoExpr {
            span,
            ty: ptr_param.ty,
            kind: MonoExprKind::Var(MonoId {
                name: ptr_param.name.clone(),
                ty: ptr_param.ty,
                span,
            }),
        };
        let pat = MonoPat {
            span,
            ty: MonoTy::new_unchecked(*decoder),
            kind: MonoPatKind::Con {
                ctor: MonoId {
                    name: decoder_ctor_name,
                    ty: MonoTy::new_unchecked(*decoder),
                    span,
                },
                args: vec![MonoPat {
                    span,
                    ty: MonoTy::new_unchecked(*reader),
                    kind: MonoPatKind::Var(rdr_id),
                }],
            },
        };
        Some(MonoFunction {
            origin: MonoFunctionOrigin::DerivedGeneric {
                adt: key.adt,
                method: "ABIDecode.decode".to_owned(),
            },
            source: None,
            shadowed_top_level: None,
            name: name.to_owned(),
            span,
            params: vec![ptr_param, head_param],
            ret: MonoTy::new_unchecked(*decoded),
            comptime_obligations: Vec::new(),
            body: vec![MonoStmt {
                span,
                kind: MonoStmtKind::Match {
                    scrutinees: vec![ptr],
                    arms: vec![MonoArm {
                        span,
                        pats: vec![pat],
                        body: vec![MonoStmt {
                            span,
                            kind: MonoStmtKind::Return(Some(result)),
                        }],
                    }],
                },
            }],
        })
    }

    fn solve_derived_abi_pred_with_givens(
        &mut self,
        key: &DerivedAbiKey<'db>,
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
