use chumsky::{input::ValueInput, prelude::*};
use hir::ast::item::FuncKind;

use super::{
    common::*,
    expr_pat::parsed_expr_parser,
    imports::{export_parser, import_parser, pragma_parser},
    recovery::trace_recovery,
    types::{pred_list_parser, type_parser},
};
use crate::{lexer::Token, types::*};

fn param_parser<'src, I>() -> impl Parser<'src, I, ParsedFuncParam<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let comptime_typed = comptime_kw_parser()
        .then(ident_parser())
        .then_ignore(just(Token::Colon))
        // First probe the longer `comptime name: Type` shape. Rewinding keeps
        // the actual parser branch from consuming input during the lookahead.
        .rewind()
        .ignore_then(comptime_kw_parser())
        .then(ident_parser())
        .then_ignore(just(Token::Colon))
        .then(type_parser())
        .map(|((comptime, name), ty)| ParsedFuncParam::Typed {
            comptime: Some(comptime),
            name,
            ty,
        })
        .boxed();

    let param_end = just(Token::Comma).or(just(Token::RParen)).ignored();
    let comptime_untyped = comptime_kw_parser()
        .then(ident_parser())
        .then_ignore(param_end.rewind())
        // `comptime name` is accepted only at a parameter boundary; otherwise
        // `comptime name: Type` must be parsed by the typed branch above.
        .rewind()
        .ignore_then(comptime_kw_parser())
        .then(ident_parser())
        .map(|(comptime, name)| ParsedFuncParam::Untyped {
            comptime: Some(comptime),
            name,
        })
        .boxed();

    let typed = non_comptime_param_name_parser()
        .then_ignore(just(Token::Colon))
        .then(type_parser())
        .map(|(name, ty)| ParsedFuncParam::Typed {
            comptime: None,
            name,
            ty,
        })
        .boxed();

    let untyped = non_comptime_param_name_parser()
        .map(|name| ParsedFuncParam::Untyped {
            comptime: None,
            name,
        })
        .boxed();

    let recovery = any()
        .and_is(just(Token::Comma).not())
        .and_is(just(Token::RParen).not())
        .repeated()
        .at_least(1)
        .map_with(|_, e| {
            let span = e.span();
            trace_recovery("function_param", span);
            ParsedFuncParam::Error { span }
        });

    choice((comptime_typed, comptime_untyped, typed, untyped))
        .validate(|param, _, emitter| {
            if let ParsedFuncParam::Typed { ty, .. } = &param
                && matches!(ty.kind, ParsedTyKind::Comptime { .. })
            {
                emitter.emit(Rich::custom(
                    ty.span,
                    "`comptime<T>` is not a parameter type; write `comptime name: T`",
                ));
            }
            param
        })
        .recover_with(via_parser(recovery))
        .labelled("function parameter")
        .as_context()
}

/// Parses a parameter of a named function-like declaration.
///
/// Named functions, trait methods, constructors, and fallbacks require an
/// explicit type for every parameter. Keeping the untyped shape as an error
/// node lets parsing recover at the following comma without exposing inferred
/// named parameters to later semantic phases.
fn named_param_parser<'src, I>() -> impl Parser<'src, I, ParsedFuncParam<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    param_parser().validate(|param, extra, emitter| match param {
        ParsedFuncParam::Untyped { .. } => {
            let span = extra.span();
            emitter.emit(Rich::custom(
                span,
                "named function parameter requires an explicit type",
            ));
            ParsedFuncParam::Error { span }
        }
        param => param,
    })
}

/// Parses a lambda parameter.
///
/// Ordinary lambda parameters may omit their type for inference. A `comptime`
/// parameter is still required to carry an explicit type.
pub(super) fn lambda_param_parser<'src, I>()
-> impl Parser<'src, I, ParsedFuncParam<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    param_parser().validate(|param, extra, emitter| match param {
        ParsedFuncParam::Untyped {
            comptime: Some(_), ..
        } => {
            let span = extra.span();
            emitter.emit(Rich::custom(
                span,
                "`comptime` parameter requires an explicit type",
            ));
            ParsedFuncParam::Error { span }
        }
        param => param,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct ParsedFuncModifiers {
    public: Option<LexSpan>,
    payable: Option<LexSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FunctionContext {
    Module,
    Contract,
}

impl FunctionContext {
    fn allows_contract_modifiers(self) -> bool {
        matches!(self, Self::Contract)
    }
}

fn generic_param_list_parser<'src, I>()
-> impl Parser<'src, I, (Vec<SpannedStr<'src>>, LexSpan), ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    ident_parser()
        .separated_by(just(Token::Comma))
        .at_least(1)
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::Less), just(Token::Greater))
        .map_with(|params, e| (params, e.span()))
}

fn optional_generic_params_parser<'src, I>()
-> impl Parser<'src, I, (Vec<SpannedStr<'src>>, Option<LexSpan>), ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    generic_param_list_parser()
        .or_not()
        .map(|params| match params {
            Some((params, span)) => (params, Some(span)),
            None => (Vec::new(), None),
        })
}

fn return_type_parser<'src, I>() -> impl Parser<'src, I, ParsedTy<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    type_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map_with(|elems, e| match <[_; 1]>::try_from(elems) {
            Ok([elem]) => elem,
            Err(elems) => ParsedTy {
                span: e.span(),
                kind: ParsedTyKind::Tuple { elems },
            },
        })
}

fn where_clause_parser<'src, I>() -> impl Parser<'src, I, Vec<ParsedPred<'src>>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    where_kw_parser()
        .ignore_then(pred_list_parser())
        .or_not()
        .map(Option::unwrap_or_default)
}

fn parsed_ident_type<'src>(ident: SpannedStr<'src>) -> ParsedTy<'src> {
    ParsedTy {
        span: ident.1,
        kind: ParsedTyKind::Named {
            qualifiers: Vec::new(),
            name: ident,
            args: Vec::new(),
            args_span: None,
        },
    }
}

fn contract_modifiers_parser<'src, I>(
    context: FunctionContext,
) -> impl Parser<'src, I, ParsedFuncModifiers, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let public = just(Token::Public).map_with(|_, e| e.span()).or_not();
    let payable = just(Token::Payable).map_with(|_, e| e.span()).or_not();

    public
        .then(payable)
        .validate(move |(public, payable), _, emitter| {
            if !context.allows_contract_modifiers() {
                if let Some(span) = public {
                    emitter.emit(Rich::custom(
                        span,
                        "'public' is only allowed on functions declared inside a contract",
                    ));
                }
                if let Some(span) = payable {
                    emitter.emit(Rich::custom(
                        span,
                        "`payable` is only allowed on a function, constructor, or fallback inside a contract",
                    ));
                }
            }
            ParsedFuncModifiers { public, payable }
        })
}

fn implicit_public_modifiers_parser<'src, I>(
    context: FunctionContext,
    decl_name: &'static str,
) -> impl Parser<'src, I, ParsedFuncModifiers, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let public = just(Token::Public).map_with(|_, e| e.span()).or_not();
    let payable = just(Token::Payable).map_with(|_, e| e.span()).or_not();

    public
        .then(payable)
        .validate(move |(public, payable), _, emitter| {
            if let Some(span) = public {
                emitter.emit(Rich::custom(
                    span,
                    format!("{decl_name} is implicitly public; remove the 'public' keyword"),
                ));
            }
            if !context.allows_contract_modifiers()
                && let Some(span) = payable
            {
                emitter.emit(Rich::custom(
                    span,
                    "`payable` is only allowed on a function, constructor, or fallback inside a contract",
                ));
            }
            ParsedFuncModifiers {
                public: None,
                payable,
            }
        })
}

fn signature_parser<'src, I>(
    context: FunctionContext,
) -> impl Parser<'src, I, ParsedFuncSig<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let modifiers = contract_modifiers_parser(context).boxed();

    let params = named_param_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map_with(|params, e| (params, e.span()))
        .boxed();

    let ret = returns_kw_parser()
        .ignore_then(return_type_parser())
        .or_not()
        .boxed();

    just(Token::Function)
        .ignore_then(ident_parser())
        .then(optional_generic_params_parser())
        .then(params)
        .then(modifiers)
        .then(ret)
        .then(where_clause_parser())
        .map_with(
            |(((((name, (type_vars, _)), (params, params_span)), modifiers), ret), preds), e| {
                let ret = Some(ret.unwrap_or_else(|| ParsedTy {
                    span: e.span(),
                    kind: ParsedTyKind::Tuple { elems: Vec::new() },
                }));
                ParsedFuncSig {
                    span: e.span(),
                    type_vars,
                    preds,
                    public: modifiers.public,
                    payable: modifiers.payable,
                    name,
                    params,
                    params_span,
                    ret,
                }
            },
        )
        .labelled("function signature")
        .as_context()
        .boxed()
}

pub(super) fn body_span_parser<'src, I>() -> impl Parser<'src, I, LexSpan, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let body_contents = recursive(|body_contents| {
        let nested = body_contents
            .clone()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .ignored();

        choice((
            nested,
            any()
                .and_is(just(Token::LBrace).not())
                .and_is(just(Token::RBrace).not())
                .ignored(),
        ))
        .repeated()
        .ignored()
    });

    just(Token::LBrace)
        .ignore_then(body_contents)
        .then_ignore(just(Token::RBrace))
        .map_with(|_, e| e.span())
}

fn function_def_parser<'src, I>(
    context: FunctionContext,
) -> impl Parser<'src, I, ParsedFunctionDef<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    signature_parser(context)
        .then(body_span_parser())
        .map_with(|(sig, body_span), e| ParsedFunctionDef {
            span: e.span(),
            kind: FuncKind::Function,
            leading_comments: Vec::new(),
            sig,
            body_span,
        })
        .labelled("function definition")
        .as_context()
        .boxed()
}

fn constructor_def_parser<'src, I>(
    context: FunctionContext,
) -> impl Parser<'src, I, ParsedFunctionDef<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let modifiers = implicit_public_modifiers_parser(context, "constructor").boxed();
    let params = named_param_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map_with(|params, e| (params, e.span()))
        .boxed();

    just(Token::Constructor)
        .map_with(|_, e| e.span())
        .then(params)
        .then(modifiers)
        .then(body_span_parser())
        .map_with(
            |(((name_span, (params, params_span)), modifiers), body_span), e| ParsedFunctionDef {
                span: e.span(),
                kind: FuncKind::Constructor,
                leading_comments: Vec::new(),
                sig: ParsedFuncSig {
                    span: e.span(),
                    type_vars: Vec::new(),
                    preds: Vec::new(),
                    public: modifiers.public,
                    payable: modifiers.payable,
                    name: ("constructor", name_span),
                    params,
                    params_span,
                    ret: None,
                },
                body_span,
            },
        )
        .labelled("constructor definition")
        .as_context()
        .boxed()
}

fn fallback_def_parser<'src, I>(
    context: FunctionContext,
) -> impl Parser<'src, I, ParsedFunctionDef<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let modifiers = implicit_public_modifiers_parser(context, "fallback").boxed();

    let params = named_param_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .map_with(|params, e| (params, e.span()))
        .boxed();

    just(Token::Fallback)
        .map_with(|_, e| e.span())
        .then(params)
        .validate(|value, _, emitter| {
            let (_, (params, params_span)) = &value;
            if !params.is_empty() {
                emitter.emit(Rich::custom(
                    *params_span,
                    "fallback function must not declare input parameters",
                ));
            }
            value
        })
        .then(modifiers)
        .then(body_span_parser())
        .map_with(
            |(((name_span, (params, params_span)), modifiers), body_span), e| ParsedFunctionDef {
                span: e.span(),
                kind: FuncKind::Fallback,
                leading_comments: Vec::new(),
                sig: ParsedFuncSig {
                    span: e.span(),
                    type_vars: Vec::new(),
                    preds: Vec::new(),
                    public: modifiers.public,
                    payable: modifiers.payable,
                    name: ("fallback", name_span),
                    params,
                    params_span,
                    ret: None,
                },
                body_span,
            },
        )
        .labelled("fallback definition")
        .as_context()
        .boxed()
}

fn function_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    function_def_parser(FunctionContext::Module)
        .map(|def| ParsedTopItem::Function {
            span: def.span,
            leading_comments: def.leading_comments,
            sig: def.sig,
            body_span: def.body_span,
        })
        .labelled("function declaration")
        .as_context()
        .boxed()
}

fn type_alias_payload_parser<'src, I>()
-> impl Parser<'src, I, (SpannedStr<'src>, Vec<SpannedStr<'src>>, ParsedTy<'src>), ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let ty_params = ident_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|params| params.unwrap_or_default())
        .boxed();

    let type_recovery = any()
        .and_is(just(Token::Semi).not())
        .repeated()
        .at_least(1)
        .map_with(|_, e| {
            let span = e.span();
            trace_recovery("type_alias_type", span);
            ParsedTy {
                span,
                kind: ParsedTyKind::Error,
            }
        });

    just(Token::Type)
        .ignore_then(ident_parser())
        .then(ty_params)
        .then_ignore(just(Token::Eq))
        .then(type_parser().recover_with(via_parser(type_recovery)))
        .then_ignore(just(Token::Semi))
        .map(|((name, ty_params), ty)| (name, ty_params, ty))
}

fn type_alias_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    type_alias_payload_parser()
        .map_with(|(name, ty_params, ty), e| ParsedTopItem::TypeAlias {
            span: e.span(),
            leading_comments: Vec::new(),
            name,
            ty_params,
            ty,
        })
        .labelled("type alias declaration")
        .as_context()
        .boxed()
}

fn data_ctor_parser<'src, I>() -> impl Parser<'src, I, ParsedAdtCtor<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let fields = type_parser()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|fields| fields.unwrap_or_default());

    ident_parser()
        .then(fields)
        .map_with(|(name, fields), e| ParsedAdtCtor {
            span: e.span(),
            // Filled by `adt_payload_parser`, which owns the introducing
            // `{`/`,` token.
            introducer: None,
            leading_comments: Vec::new(),
            name,
            fields,
        })
        .boxed()
}

fn derive_target_parser<'src, I>() -> impl Parser<'src, I, ParsedDeriveTarget<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let ident = select! {
        Token::Ident(name) => (name, false),
        Token::Import => ("import", true),
        Token::Export => ("export", true),
        Token::Pragma => ("pragma", true),
        Token::Type => ("type", true),
        Token::Data => ("data", true),
        Token::Class => ("class", true),
        Token::Instance => ("instance", true),
        Token::Contract => ("contract", true),
        Token::Public => ("public", true),
        Token::Payable => ("payable", true),
        Token::Function => ("function", true),
        Token::Constructor => ("constructor", true),
        Token::Fallback => ("fallback", true),
        Token::Forall => ("forall", true),
        Token::Default => ("default", true),
    }
    .validate(|(name, reserved), e, emitter| {
        if reserved {
            emitter.emit(Rich::custom(
                e.span(),
                format!("reserved keyword `{name}` cannot name a derived trait"),
            ));
        }
        if name.contains('-') {
            emitter.emit(Rich::custom(
                e.span(),
                format!("identifier `{name}` cannot contain hyphens"),
            ));
        }
        (name, e.span())
    });
    ident
        .separated_by(just(Token::Dot))
        .at_least(1)
        .collect::<Vec<_>>()
        .map_with(|path, e| ParsedDeriveTarget {
            span: e.span(),
            path,
        })
}

fn derive_declaration_boundary_parser<'src, I>() -> impl Parser<'src, I, (), ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let contract_field = ident_parser()
        .then_ignore(just(Token::Colon))
        .rewind()
        .ignored();
    just(Token::RBrace)
        .to(())
        .or(top_level_item_start_token_parser())
        .or(contract_field)
}

fn derive_attr_parser<'src, I>() -> impl Parser<'src, I, ParsedDeriveAttr<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let derive_kw = select! { Token::Ident(name) if name == "derive" => () };
    let targets = derive_target_parser()
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen));
    let valid = just(Token::Hash)
        .ignore_then(just(Token::LBracket))
        .ignore_then(derive_kw)
        .ignore_then(targets)
        .then_ignore(just(Token::RBracket))
        .map_with(|targets, e| ParsedDeriveAttr {
            span: e.span(),
            targets,
        })
        .validate(|attr, _, emitter| {
            if attr.targets.is_empty() {
                emitter.emit(Rich::custom(
                    attr.span,
                    "derive attribute requires at least one trait path",
                ));
            }
            attr
        });

    // Once `#[` has been seen, consume a closed but otherwise malformed
    // attribute as one recoverable unit. This keeps the following declaration
    // available to the ordinary item parser.
    let malformed_boundary = derive_declaration_boundary_parser();
    let malformed = just(Token::Hash)
        .ignore_then(just(Token::LBracket))
        .ignore_then(
            any()
                .and_is(just(Token::RBracket).not())
                .and_is(malformed_boundary.not())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::RBracket))
        .map_with(|_, e| ParsedDeriveAttr {
            span: e.span(),
            targets: Vec::new(),
        })
        .validate(|attr, _, emitter| {
            emitter.emit(Rich::custom(
                attr.span,
                "malformed derive attribute; expected `#[derive(Trait, ...)]`",
            ));
            attr
        });

    // If the closing `]` is missing, stop before the next declaration so the
    // outer item parser can still recover that declaration. `RBrace` is also a
    // boundary for contract-local attributes.
    let recovery_boundary = just(Token::RBracket)
        .to(())
        .or(derive_declaration_boundary_parser());
    let unclosed = just(Token::Hash)
        .ignore_then(just(Token::LBracket))
        .ignore_then(
            any()
                .and_is(recovery_boundary.not())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map_with(|_, e| ParsedDeriveAttr {
            span: e.span(),
            targets: Vec::new(),
        })
        .validate(|attr, _, emitter| {
            emitter.emit(Rich::custom(
                attr.span,
                "unclosed derive attribute; expected `]`",
            ));
            attr
        });

    valid
        .or(malformed)
        .or(unclosed)
        .labelled("derive attribute")
        .as_context()
        .boxed()
}

fn adt_payload_parser<'src, I>() -> impl Parser<
    'src,
    I,
    (
        SpannedStr<'src>,
        Vec<SpannedStr<'src>>,
        Vec<ParsedAdtCtor<'src>>,
    ),
    ParserErr<'src>,
>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let following_ctor = just(Token::Comma)
        .map_with(|_, e| e.span())
        .then(data_ctor_parser())
        .map(|(introducer, mut ctor)| {
            ctor.introducer = Some(introducer);
            ctor
        });
    let ctor_list = data_ctor_parser()
        .then(following_ctor.repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::Comma).or_not())
        .map(|(first, mut rest)| {
            let mut ctors = Vec::with_capacity(rest.len() + 1);
            ctors.push(first);
            ctors.append(&mut rest);
            ctors
        });
    let ctors = just(Token::LBrace)
        .map_with(|_, e| e.span())
        .then(ctor_list.or_not())
        .then_ignore(just(Token::RBrace))
        .map(|(introducer, ctors)| {
            let mut ctors = ctors.unwrap_or_default();
            if let Some(first) = ctors.first_mut() {
                first.introducer = Some(introducer);
            }
            ctors
        })
        .boxed();

    enum_kw_parser()
        .ignore_then(ident_parser())
        .then(optional_generic_params_parser())
        .then(ctors)
        .map(|((name, (ty_params, _)), ctors)| (name, ty_params, ctors))
}

fn adt_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    adt_payload_parser()
        .map_with(|(name, ty_params, ctors), e| ParsedTopItem::Adt {
            span: e.span(),
            leading_comments: Vec::new(),
            derive_attr: None,
            name,
            ty_params,
            ctors,
        })
        .labelled("enum declaration")
        .as_context()
        .boxed()
}

fn method_sig_parser<'src, I>() -> impl Parser<'src, I, ParsedClassMethod<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    signature_parser(FunctionContext::Module)
        .then_ignore(just(Token::Semi))
        .map(|sig| ParsedClassMethod {
            leading_comments: Vec::new(),
            sig,
        })
        .boxed()
}

fn class_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let methods = method_sig_parser()
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .boxed();

    trait_kw_parser()
        .ignore_then(ident_parser())
        .then(generic_param_list_parser())
        .then(where_clause_parser())
        .then(methods)
        .map_with(
            |(((name, (type_vars, args_span)), super_preds), methods), e| {
                let mut head_types = type_vars.iter().copied().map(parsed_ident_type);
                let subject = head_types
                    .next()
                    .expect("trait generic parameter parser is non-empty");
                let args = head_types.collect::<Vec<_>>();
                let head = ParsedPred {
                    ty: subject,
                    class: name,
                    args,
                    args_span: Some(args_span),
                };
                ParsedTopItem::Class {
                    span: e.span(),
                    leading_comments: Vec::new(),
                    type_vars,
                    super_preds,
                    head,
                    methods,
                }
            },
        )
        .labelled("trait declaration")
        .as_context()
        .boxed()
}

fn instance_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let default_kw = just(Token::Default)
        .map_with(|_, e| e.span())
        .or_not()
        .boxed();

    let methods = function_def_parser(FunctionContext::Module)
        .repeated()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LBrace), just(Token::RBrace))
        .boxed();

    let head = ident_parser()
        .then(
            type_parser()
                .separated_by(just(Token::Comma))
                .at_least(1)
                .allow_trailing()
                .collect::<Vec<_>>()
                .delimited_by(just(Token::Less), just(Token::Greater))
                .map_with(|args, e| (args, e.span())),
        )
        .map(|(class, (mut args, args_span))| {
            let ty = args.remove(0);
            ParsedPred {
                ty,
                class,
                args,
                args_span: Some(args_span),
            }
        })
        .boxed();

    default_kw
        .then_ignore(impl_kw_parser())
        .then(optional_generic_params_parser())
        .then(head)
        .then(where_clause_parser())
        .then(methods)
        .map_with(
            |((((default_kw, (type_vars, _)), head), preds), methods), e| ParsedTopItem::Instance {
                span: e.span(),
                leading_comments: Vec::new(),
                type_vars,
                preds,
                default_kw,
                head,
                methods,
            },
        )
        .labelled("impl declaration")
        .as_context()
        .boxed()
}

fn field_def_parser<'src, I>() -> impl Parser<'src, I, ParsedFieldDef<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    ident_parser()
        .then_ignore(just(Token::Colon))
        .rewind()
        .ignore_then(ident_parser())
        .then_ignore(just(Token::Colon))
        .then(type_parser())
        .then(just(Token::Eq).ignore_then(parsed_expr_parser()).or_not())
        .then_ignore(just(Token::Semi))
        .map_with(|((name, ty), init), e| ParsedFieldDef {
            span: e.span(),
            leading_comments: Vec::new(),
            name,
            ty,
            init,
        })
        .labelled("contract field")
        .as_context()
        .boxed()
}

#[derive(Debug, Clone)]
enum ParsedContractMember<'src> {
    Field(ParsedFieldDef<'src>),
    Item(ParsedContractItem<'src>),
}

fn contract_item_parser<'src, I>() -> impl Parser<'src, I, ParsedContractItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let function_def = function_def_parser(FunctionContext::Contract)
        .map(ParsedContractItem::Function)
        .boxed();
    let constructor_def = constructor_def_parser(FunctionContext::Contract)
        .map(ParsedContractItem::Function)
        .boxed();
    let fallback_def = fallback_def_parser(FunctionContext::Contract)
        .map(ParsedContractItem::Function)
        .boxed();

    let type_alias = type_alias_payload_parser()
        .map_with(|(name, ty_params, ty), e| ParsedContractItem::TypeAlias {
            span: e.span(),
            leading_comments: Vec::new(),
            name,
            ty_params,
            ty,
        })
        .boxed();

    let adt_def = adt_payload_parser()
        .map_with(|(name, ty_params, ctors), e| ParsedContractItem::Adt {
            span: e.span(),
            leading_comments: Vec::new(),
            derive_attr: None,
            name,
            ty_params,
            ctors,
        })
        .boxed();

    let item_start = choice((
        select! {
            Token::Hash | Token::Function | Token::Constructor | Token::Fallback
            | Token::Type | Token::RBrace => (),
        },
        enum_kw_parser().ignored(),
    ));
    let recovery = any()
        .and_is(item_start.not())
        .repeated()
        .at_least(1)
        .map_with(|_, e| {
            let span = e.span();
            trace_recovery("contract_member", span);
            ParsedContractItem::Error {
                span,
                leading_comments: Vec::new(),
            }
        });

    choice((
        function_def,
        constructor_def,
        fallback_def,
        type_alias,
        adt_def,
    ))
    .recover_with(via_parser(recovery))
    .labelled("contract member")
    .as_context()
}

fn contract_member_parser<'src, I>()
-> impl Parser<'src, I, ParsedContractMember<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let member = field_def_parser()
        .map(ParsedContractMember::Field)
        .or(contract_item_parser().map(ParsedContractMember::Item))
        .boxed();

    derive_attr_parser()
        .or_not()
        .then(member)
        .validate(|(derive_attr, mut member), _, emitter| {
            let Some(attr) = derive_attr else {
                return member;
            };

            match &mut member {
                ParsedContractMember::Item(ParsedContractItem::Adt {
                    span, derive_attr, ..
                }) => {
                    *span = LexSpan::from(attr.span.start..span.end);
                    *derive_attr = Some(attr);
                }
                _ => {
                    emitter.emit(Rich::custom(
                        attr.span,
                        "derive attribute is only allowed on enum declarations",
                    ));
                    let span = match &mut member {
                        ParsedContractMember::Field(field) => &mut field.span,
                        ParsedContractMember::Item(ParsedContractItem::Function(function)) => {
                            &mut function.span
                        }
                        ParsedContractMember::Item(ParsedContractItem::TypeAlias {
                            span, ..
                        })
                        | ParsedContractMember::Item(ParsedContractItem::Adt { span, .. })
                        | ParsedContractMember::Item(ParsedContractItem::Error { span, .. }) => {
                            span
                        }
                    };
                    *span = LexSpan::from(attr.span.start..span.end);
                }
            }
            member
        })
        .boxed()
}

fn contract_parser<'src, I>() -> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let members = contract_member_parser()
        .repeated()
        .collect::<Vec<_>>()
        .boxed();
    let body = members.delimited_by(just(Token::LBrace), just(Token::RBrace));

    just(Token::Contract)
        .ignore_then(ident_parser())
        .then(optional_generic_params_parser())
        .then(body)
        .map_with(|((name, (ty_params, _)), members), e| {
            let mut fields = Vec::new();
            let mut items = Vec::new();
            for member in members {
                match member {
                    ParsedContractMember::Field(field) => fields.push(field),
                    ParsedContractMember::Item(item) => items.push(item),
                }
            }
            ParsedTopItem::Contract {
                span: e.span(),
                leading_comments: Vec::new(),
                name,
                ty_params,
                fields,
                items,
            }
        })
        .labelled("contract declaration")
        .as_context()
        .boxed()
}

pub(super) fn top_item_parser<'src, I>()
-> impl Parser<'src, I, ParsedTopItem<'src>, ParserErr<'src>>
where
    I: ValueInput<'src, Token = Token<'src>, Span = LexSpan>,
{
    let item_start = top_level_item_start_token_parser();
    let recovery = any()
        .and_is(item_start.not())
        .repeated()
        .at_least(1)
        .map_with(|_, e| {
            let span = e.span();
            trace_recovery("top_level_item", span);
            ParsedTopItem::Error {
                span,
                leading_comments: Vec::new(),
            }
        });

    let item = choice((
        import_parser(),
        export_parser(),
        pragma_parser(),
        type_alias_parser(),
        adt_parser(),
        class_parser(),
        instance_parser(),
        contract_parser(),
        function_parser(),
    ));

    derive_attr_parser()
        .or_not()
        .then(item)
        .validate(|(derive_attr, mut item), _, emitter| {
            let Some(attr) = derive_attr else {
                return item;
            };

            match &mut item {
                ParsedTopItem::Adt {
                    span, derive_attr, ..
                } => {
                    *span = LexSpan::from(attr.span.start..span.end);
                    *derive_attr = Some(attr);
                }
                _ => {
                    emitter.emit(Rich::custom(
                        attr.span,
                        "derive attribute is only allowed on enum declarations",
                    ));
                    let span = match &mut item {
                        ParsedTopItem::Import { span, .. }
                        | ParsedTopItem::Export { span, .. }
                        | ParsedTopItem::Pragma { span, .. }
                        | ParsedTopItem::TypeAlias { span, .. }
                        | ParsedTopItem::Adt { span, .. }
                        | ParsedTopItem::Class { span, .. }
                        | ParsedTopItem::Instance { span, .. }
                        | ParsedTopItem::Contract { span, .. }
                        | ParsedTopItem::Function { span, .. }
                        | ParsedTopItem::Error { span, .. } => span,
                    };
                    *span = LexSpan::from(attr.span.start..span.end);
                }
            }
            item
        })
        .recover_with(via_parser(recovery))
        .labelled("top-level item")
        .as_context()
}
