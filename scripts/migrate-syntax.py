#!/usr/bin/env python3
"""Migrate tracked Solcore sources to the syntax documented in syntax.md.

The repository is in the middle of changing its source extension.  The input
set is therefore the tracked ``*.solc`` paths, but a missing input is resolved
to its sibling ``*.sol`` file.  This makes the script safe to rerun before or
after the extension-only migration.

The transformer is deliberately token based.  It does not touch comments,
strings, or the contents of ``assembly { ... }`` blocks.  Intentional
syntax-error fixtures are migrated too: the malformed construct is retained
where possible, while independent surrounding declarations are canonicalized.
"""

from __future__ import annotations

import argparse
import dataclasses
import pathlib
import re
import subprocess
import sys
from collections.abc import Iterable, Sequence


ROOT = pathlib.Path(__file__).resolve().parents[1]
REPORT = ROOT / "scripts" / "migrate-syntax-unsafe.txt"


@dataclasses.dataclass(frozen=True)
class Tok:
    text: str
    start: int
    end: int


TOKEN_RE = re.compile(
    r"""
    (?P<space>\s+)
  | (?P<line>//[^\n]*)
  | (?P<block>/\*.*?\*/)
  | (?P<string>"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')
  | (?P<ident>[A-Za-z_$][A-Za-z0-9_$-]*)
  | (?P<number>0[xX][0-9A-Fa-f]+|[0-9]+)
  | (?P<op>:=|->|=>|==|!=|>=|<=|&&|\|\||\+=|-=|\*=|/=|\^=|&=|\|=|%=|~=)
  | (?P<punct>.)
    """,
    re.S | re.X,
)


def tokens(source: str) -> list[Tok]:
    out: list[Tok] = []
    for match in TOKEN_RE.finditer(source):
        if match.lastgroup in {"space", "line", "block", "string"}:
            continue
        out.append(Tok(match.group(), match.start(), match.end()))
    return out


def pairs(ts: Sequence[Tok]) -> tuple[dict[int, int], dict[int, int]]:
    # Angle tokens are ambiguous outside a known type context: they are also
    # comparison operators.  Pairing them globally makes `i < n` hide later
    # braces and declarations from the migrator.
    opens = {"(": ")", "[": "]", "{": "}"}
    close_to_open = {value: key for key, value in opens.items()}
    stack: list[tuple[str, int]] = []
    forward: dict[int, int] = {}
    backward: dict[int, int] = {}
    for i, tok in enumerate(ts):
        if tok.text in opens:
            stack.append((tok.text, i))
        elif tok.text in close_to_open:
            wanted = close_to_open[tok.text]
            # Invalid fixtures can contain unmatched delimiters.  Pair only a
            # well-nested suffix and let the caller classify the file unsafe.
            if stack and stack[-1][0] == wanted:
                _, j = stack.pop()
                forward[j] = i
                backward[i] = j
    return forward, backward


def split_top(
    ts: Sequence[Tok], separator: str = ",", *, angles: bool = True
) -> list[list[Tok]]:
    result: list[list[Tok]] = []
    current: list[Tok] = []
    depth = {"(": 0, "[": 0, "{": 0}
    close = {")": "(", "]": "[", "}": "{"}
    if angles:
        depth["<"] = 0
        close[">"] = "<"
    for tok in ts:
        if tok.text == separator and not any(depth.values()):
            result.append(current)
            current = []
            continue
        current.append(tok)
        if tok.text in depth:
            depth[tok.text] += 1
        elif tok.text in close and depth[close[tok.text]]:
            depth[close[tok.text]] -= 1
    result.append(current)
    return result


def top_index(
    ts: Sequence[Tok], choices: set[str], *, angles: bool = True
) -> int | None:
    depth = {"(": 0, "[": 0, "{": 0}
    close = {")": "(", "]": "[", "}": "{"}
    if angles:
        depth["<"] = 0
        close[">"] = "<"
    for i, tok in enumerate(ts):
        if tok.text in choices and not any(depth.values()):
            return i
        if tok.text in depth:
            depth[tok.text] += 1
        elif tok.text in close and depth[close[tok.text]]:
            depth[close[tok.text]] -= 1
    return None


def source_slice(source: str, ts: Sequence[Tok]) -> str:
    return "" if not ts else source[ts[0].start : ts[-1].end]


def apply_edits(source: str, edits: Iterable[tuple[int, int, str]]) -> str:
    ordered = sorted(edits, key=lambda edit: (edit[0], edit[1]))
    for left, right in zip(ordered, ordered[1:]):
        if left[1] > right[0]:
            raise ValueError(f"overlapping migration edits: {left[:2]} and {right[:2]}")
    for start, end, replacement in reversed(ordered):
        source = source[:start] + replacement + source[end:]
    return source


def assembly_ranges(ts: Sequence[Tok]) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    for i, tok in enumerate(ts[:-1]):
        if tok.text != "assembly" or ts[i + 1].text != "{":
            continue
        # Pair assembly braces independently of parentheses/brackets.  Parser
        # recovery fixtures intentionally contain an unclosed Yul call; the
        # surrounding assembly block is still delimited and must remain fully
        # opaque to every Core syntax transform.
        depth = 1
        j = i + 2
        while j < len(ts):
            if ts[j].text == "{":
                depth += 1
            elif ts[j].text == "}":
                depth -= 1
                if depth == 0:
                    ranges.append((i + 1, j))
                    break
            j += 1
    return ranges


def in_ranges(index: int, ranges: Sequence[tuple[int, int]]) -> bool:
    return any(start <= index <= end for start, end in ranges)


class TypeParser:
    def __init__(self, ts: Sequence[Tok]):
        self.ts = list(ts)
        self.i = 0

    def take(self, text: str | None = None) -> Tok:
        if self.i >= len(self.ts):
            raise ValueError("unexpected end of type")
        tok = self.ts[self.i]
        if text is not None and tok.text != text:
            raise ValueError(f"expected {text!r}, found {tok.text!r}")
        self.i += 1
        return tok

    def parse(self):
        left = self.atom()
        if self.i < len(self.ts) and self.ts[self.i].text == "->":
            self.i += 1
            return ("fn-old", left, self.parse())
        return left

    def atom(self):
        tok = self.take()
        if tok.text == "comptime":
            if self.i == len(self.ts):
                return ("named", "comptime", [])
            if self.i < len(self.ts) and self.ts[self.i].text == "<":
                self.i += 1
                inner = self.parse()
                self.take(">")
            else:
                inner = self.atom()
            return ("comptime", inner)
        if tok.text == "@":
            return ("proxy", self.parse())
        if tok.text == "function":
            self.take("(")
            params = self.list_until(")")
            ret = ("tuple", [])
            if self.i < len(self.ts) and self.ts[self.i].text == "returns":
                self.i += 1
                self.take("(")
                ret = ("tuple", self.list_until(")"))
            return ("fn", params, ret)
        if tok.text == "(":
            return ("tuple", self.list_until(")"))
        if not re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$-]*", tok.text):
            raise ValueError(f"expected type name, found {tok.text!r}")
        name = tok.text
        while self.i + 1 < len(self.ts) and self.ts[self.i].text == ".":
            self.i += 1
            name += "." + self.take().text
        args = []
        if self.i < len(self.ts) and self.ts[self.i].text in {"(", "<"}:
            opener = self.take().text
            closing = ")" if opener == "(" else ">"
            if name == "mapping" and opener == "(":
                key = self.parse()
                if self.i < len(self.ts) and self.ts[self.i].text == "=>":
                    self.i += 1
                    value = self.parse()
                    self.take(")")
                    args = [key, value]
                else:
                    self.take(",")
                    value = self.parse()
                    self.take(")")
                    args = [key, value]
            else:
                args = self.list_until(closing)
        return ("named", name, args)

    def list_until(self, closing: str):
        values = []
        if self.i < len(self.ts) and self.ts[self.i].text == closing:
            self.i += 1
            return values
        while True:
            values.append(self.parse())
            if self.i < len(self.ts) and self.ts[self.i].text == ",":
                self.i += 1
                if self.i < len(self.ts) and self.ts[self.i].text == closing:
                    self.i += 1
                    return values
                continue
            self.take(closing)
            return values


def render_type_node(node) -> str:
    kind = node[0]
    if kind == "named":
        _, name, args = node
        if name == "mapping" and len(args) == 2:
            return f"mapping({render_type_node(args[0])} => {render_type_node(args[1])})"
        if args:
            return f"{name}<" + ", ".join(map(render_type_node, args)) + ">"
        return name
    if kind == "tuple":
        return "(" + ", ".join(map(render_type_node, node[1])) + ")"
    if kind == "proxy":
        return "@" + render_type_node(node[1])
    if kind == "comptime":
        return "comptime<" + render_type_node(node[1]) + ">"
    if kind in {"fn", "fn-old"}:
        if kind == "fn":
            params, ret = node[1], node[2]
        else:
            domain, ret = node[1], node[2]
            if domain[0] == "tuple" and len(domain[1]) != 1:
                params = domain[1]
            elif domain[0] == "tuple" and len(domain[1]) == 1:
                params = domain[1]
            else:
                params = [domain]
        text = "function(" + ", ".join(map(render_type_node, params)) + ")"
        if not (ret[0] == "tuple" and not ret[1]):
            returns = ret[1] if ret[0] == "tuple" else [ret]
            text += " returns (" + ", ".join(map(render_type_node, returns)) + ")"
        return text
    raise AssertionError(kind)


def render_type(ts: Sequence[Tok]) -> str:
    parser = TypeParser(ts)
    node = parser.parse()
    if parser.i != len(parser.ts):
        raise ValueError("trailing tokens in type: " + " ".join(t.text for t in parser.ts[parser.i :]))
    return render_type_node(node)


def unwrap_outer(ts: Sequence[Tok]) -> Sequence[Tok]:
    if len(ts) >= 2 and ts[0].text == "(" and ts[-1].text == ")":
        forward, _ = pairs(ts)
        if forward.get(0) == len(ts) - 1:
            return ts[1:-1]
    return ts


def render_predicate(ts: Sequence[Tok]) -> str:
    ts = list(unwrap_outer(ts))
    colon = top_index(ts, {":"})
    if colon is None or not ts[:colon] or not ts[colon + 1 :]:
        raise ValueError("invalid predicate")
    subject = render_type(ts[:colon])
    rhs = ts[colon + 1 :]
    name = rhs[0].text
    if len(rhs) == 1:
        return f"{subject}: {name}"
    if rhs[1].text not in {"(", "<"} or rhs[-1].text not in {")", ">"}:
        raise ValueError("invalid predicate class application")
    args = split_top(rhs[2:-1])
    return f"{subject}: {name}<" + ", ".join(render_type(arg) for arg in args if arg) + ">"


def render_predicates(ts: Sequence[Tok]) -> str:
    ts = list(unwrap_outer(ts))
    return ", ".join(render_predicate(part) for part in split_top(ts) if part)


def transform_imports(source: str, warnings: list[str]) -> str:
    ts = tokens(source)
    edits = []
    for i, tok in enumerate(ts):
        if tok.text != "import":
            continue
        j = i + 1
        depth = 0
        while j < len(ts):
            if ts[j].text in "({[<":
                depth += 1
            elif ts[j].text in ")}]>" and depth:
                depth -= 1
            elif ts[j].text == ";" and depth == 0:
                break
            j += 1
        if j == len(ts):
            warnings.append("unterminated import declaration")
            continue
        body = ts[i + 1 : j]
        if not body:
            continue
        if body[0].text == "{":
            local_forward, _ = pairs(body)
            close = local_forward.get(0)
            from_i = close + 1 if close is not None else None
            if (
                close is not None
                and any(item.text == "*" for item in body[1:close])
                and from_i is not None
                and from_i < len(body)
                and body[from_i].text == "from"
            ):
                # Wildcard selection is canonical only as `import * from M`;
                # selected names next to it are redundant because the open
                # import already brings every public name into scope.
                module = "".join(item.text for item in body[from_i + 1:])
                edits.append((tok.start, ts[j].end, f"import * from {module};"))
            continue
        if body[0].text == "*":
            continue
        hiding = next((k for k, item in enumerate(body) if item.text == "hiding"), None)
        hiding_text = ""
        if hiding is not None:
            hiding_text = " " + source_slice(source, body[hiding:]).strip()
            body = body[:hiding]
        brace = next((k for k, item in enumerate(body) if item.text == "{"), None)
        as_pos = top_index(body, {"as"})
        if brace is not None and brace > 0 and body[brace - 1].text == ".":
            forward, _ = pairs(body)
            end = forward.get(brace)
            if end is None:
                warnings.append("unbalanced selective import")
                continue
            path = "".join(item.text for item in body[: brace - 1])
            inside = body[brace + 1 : end]
            if len(inside) == 1 and inside[0].text == "*":
                replacement = f"import * from {path}{hiding_text};"
            else:
                selector = source_slice(source, inside).strip()
                replacement = f"import {{{selector}}} from {path}{hiding_text};"
            edits.append((tok.start, ts[j].end, replacement))
        elif as_pos is not None:
            path = "".join(item.text for item in body[:as_pos])
            alias = "".join(item.text for item in body[as_pos + 1 :])
            edits.append((tok.start, ts[j].end, f"import * as {alias} from {path}{hiding_text};"))
    return apply_edits(source, edits)


def transform_data(source: str, warnings: list[str]) -> str:
    ts = tokens(source)
    asm = assembly_ranges(ts)
    forward, _ = pairs(ts)
    edits = []
    for i, tok in enumerate(ts):
        if tok.text != "data" or in_ranges(i, asm):
            continue
        if i + 1 >= len(ts):
            warnings.append("incomplete data declaration")
            continue
        j = i + 2
        params: list[Tok] = []
        if j < len(ts) and ts[j].text == "(":
            end = forward.get(j)
            if end is None:
                warnings.append("unbalanced data type parameters")
                continue
            params = ts[j + 1 : end]
            j = end + 1
        while j < len(ts) and ts[j].text not in {"=", ";"}:
            j += 1
        if j == len(ts):
            warnings.append("unterminated data declaration")
            continue
        name = ts[i + 1].text
        generic = ""
        if params:
            generic = "<" + ", ".join(t.text for part in split_top(params) for t in part) + ">"
            # The old binder grammar is identifiers only; restore separators.
            generic = "<" + ", ".join("".join(t.text for t in part) for part in split_top(params) if part) + ">"
        if ts[j].text == ";":
            edits.append((tok.start, ts[j].end, f"enum {name}{generic} {{}}"))
            continue
        start_variants = j + 1
        k = start_variants
        depth = 0
        declaration_starters = {
            "class", "constructor", "contract", "data", "default", "enum",
            "error", "event", "fallback", "forall", "function", "impl",
            "import", "instance", "modifier", "payable", "pragma", "public",
            "receive", "struct", "trait", "type",
        }
        missing_semicolon = False
        while k < len(ts):
            if ts[k].text in "([<":
                depth += 1
            elif ts[k].text in ")]>":
                depth -= 1
            elif ts[k].text == ";" and depth == 0:
                break
            elif depth == 0 and (
                ts[k].text == "}"
                or (
                    k > start_variants
                    and ts[k].text in declaration_starters
                    and "\n" in source[ts[k - 1].end : ts[k].start]
                )
            ):
                # Several intentional parser failures omit the old data
                # terminator.  Stop at the next item rather than swallowing
                # the rest of the file, so that its surrounding syntax can
                # still be migrated.
                missing_semicolon = True
                break
            k += 1
        if k == len(ts):
            missing_semicolon = True
        variants = []
        variant_tokens = ts[start_variants:k]
        malformed_trailing_pipe = bool(variant_tokens and variant_tokens[-1].text == "|")
        failed = False
        for variant in split_top(variant_tokens, "|"):
            if not variant:
                continue
            vname = variant[0].text
            if len(variant) == 1:
                variants.append(vname)
            elif variant[1].text == "(" and variant[-1].text == ")":
                fields = split_top(variant[2:-1])
                try:
                    rendered = ", ".join(render_type(field) for field in fields if field)
                except ValueError as error:
                    warnings.append(f"data {name}: {error}")
                    failed = True
                    break
                variants.append(vname + "(" + rendered + ")")
            else:
                warnings.append(f"unsupported data constructor in {name}")
                failed = True
                break
        if failed:
            continue
        replacement = f"enum {name}{generic} {{ " + ", ".join(variants) + " }"
        if malformed_trailing_pipe:
            # Preserve the dedicated trailing-separator parser failure in the
            # new enum spelling.  A trailing comma would be accepted.
            replacement = replacement[:-2] + " | }"
        if missing_semicolon:
            warnings.append(f"migrated unterminated data declaration {name}")
        end = ts[k].end if k < len(ts) and ts[k].text == ";" else ts[k - 1].end
        edits.append((tok.start, end, replacement))
    return apply_edits(source, edits)


def declaration_start(ts: Sequence[Tok], keyword: int) -> int:
    # A declaration prefix contains no braces.  The first brace or semicolon
    # on the left is therefore its item/member boundary.  In particular, stop
    # at a previous function's closing `}` instead of walking through its body.
    paren_depth = 0
    bracket_depth = 0
    j = keyword - 1
    while j >= 0:
        text = ts[j].text
        if text == ")":
            paren_depth += 1
        elif text == "(" and paren_depth:
            paren_depth -= 1
        elif text == "]":
            bracket_depth += 1
        elif text == "[" and bracket_depth:
            bracket_depth -= 1
        elif paren_depth == 0 and bracket_depth == 0 and text in {";", "{", "}"}:
            return j + 1
        j -= 1
    return 0


def parse_prefix(prefix: Sequence[Tok]):
    prefix = list(prefix)
    vars_: list[str] = []
    predicates: list[Tok] = []
    modifiers: list[str] = []
    if prefix and prefix[0].text == "forall":
        dot = top_index(prefix, {"."})
        if dot is None:
            raise ValueError("forall clause has no terminator")
        binders = [part for part in split_top(prefix[1:dot]) if part]
        for binder in binders:
            # Whitespace-separated binders do not have comma tokens, so each
            # token is a binder unless a bounded-binder colon is present.
            colon = top_index(binder, {":"})
            if colon is None:
                vars_.extend(tok.text for tok in binder if tok.text != ",")
            else:
                vars_.append(binder[0].text)
                if predicates:
                    predicates.append(Tok(",", binder[0].start, binder[0].start))
                predicates.extend(binder)
        prefix = prefix[dot + 1 :]
    modifiers = [tok.text for tok in prefix if tok.text in {"public", "payable"}]
    prefix = [tok for tok in prefix if tok.text not in {"public", "payable", "default"}]
    fat = top_index(prefix, {"=>"})
    if fat is not None:
        predicates = list(prefix[:fat])
        prefix = prefix[fat + 1 :]
    if prefix:
        raise ValueError("unrecognized declaration prefix: " + " ".join(t.text for t in prefix))
    return vars_, predicates, modifiers


def parse_params(ts: Sequence[Tok]) -> str:
    rendered = []
    for param in split_top(ts):
        if not param:
            continue
        colon = top_index(param, {":"})
        if colon is None:
            rendered.append(" ".join(t.text for t in param))
            continue
        left = " ".join(t.text for t in param[:colon])
        rendered.append(f"{left}: {render_type(param[colon + 1:])}")
    return ", ".join(rendered)


def transform_declarations(source: str, warnings: list[str]) -> str:
    ts = tokens(source)
    asm = assembly_ranges(ts)
    forward, _ = pairs(ts)
    edits = []
    occupied: list[tuple[int, int]] = []
    for i, tok in enumerate(ts):
        if tok.text not in {"class", "instance", "function", "constructor", "fallback"} or in_ranges(i, asm):
            continue
        start = declaration_start(ts, i)
        if any(left < i < right for left, right in occupied):
            continue
        old = tok.text in {"class", "instance"} or any(
            item.text in {"forall", "=>", "public", "payable"} for item in ts[start:i]
        )
        if tok.text in {"function", "constructor", "fallback"}:
            # Inspect only this header: arrows in a later declaration must not
            # make an already-migrated declaration look old. Constructors and
            # fallbacks used the same legacy arrow shell as functions.
            h = i
            local_depth = 0
            while h < len(ts):
                if ts[h].text in "([<": local_depth += 1
                elif ts[h].text in ")]>" and local_depth: local_depth -= 1
                elif local_depth == 0 and ts[h].text in {"{", ";"}: break
                h += 1
            old = old or any(item.text == "->" for item in ts[i:h])
        if tok.text in {"constructor", "fallback"}:
            old = old or bool(ts[start:i])
        if not old:
            continue
        try:
            try:
                vars_, pred_ts, modifiers = parse_prefix(ts[start:i])
            except ValueError:
                # A malformed preceding item (for example, a pragma missing
                # its semicolon or a stray block-comment terminator) must not
                # keep an independent function header in legacy syntax.
                prefix = ts[start:i]
                if prefix and prefix[0].text not in {
                    "forall", "(", "public", "payable", "default"
                }:
                    start = i
                    vars_, pred_ts, modifiers = parse_prefix([])
                else:
                    raise
            if tok.text in {"class", "instance"}:
                j = i + 1
                depth = 0
                while j < len(ts):
                    if ts[j].text in "([<": depth += 1
                    elif ts[j].text in ")]>" and depth: depth -= 1
                    elif ts[j].text == "{" and depth == 0: break
                    j += 1
                if j == len(ts):
                    raise ValueError("declaration has no body")
                head = ts[i + 1:j]
                # A few older fixtures put the instance context after the
                # `instance` keyword instead of in the declaration prefix:
                # `instance (a:C) => T(a):D`.  Canonical impl syntax always
                # moves that context to a trailing where-clause.
                if tok.text == "instance":
                    fat = top_index(head, {"=>"})
                    if fat is not None:
                        inline_predicates = list(head[:fat])
                        head = head[fat + 1:]
                        if pred_ts and inline_predicates:
                            pred_ts = list(pred_ts) + [
                                Tok(",", inline_predicates[0].start, inline_predicates[0].start)
                            ] + inline_predicates
                        elif inline_predicates:
                            pred_ts = inline_predicates
                if tok.text == "class":
                    pred = render_predicate(head)
                    subject, rhs = pred.split(": ", 1)
                    trait_name = rhs.split("<", 1)[0]
                    if not vars_:
                        # This is only lossless when every head component is a
                        # bare variable; semantic fail fixtures often violate it.
                        vars_ = [subject]
                        if "<" in rhs:
                            vars_.extend(rhs[rhs.index("<") + 1 : -1].split(", "))
                    if not all(re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$-]*", v) for v in vars_):
                        raise ValueError("trait head is not representable by generic binders")
                    replacement = "trait " + trait_name + "<" + ", ".join(vars_) + ">"
                else:
                    default = "default " if "default" in (item.text for item in ts[start:i]) else ""
                    # parse_prefix does not consume default.
                    clean_prefix = [item for item in ts[start:i] if item.text != "default"]
                    vars_, pred_ts, modifiers = parse_prefix(clean_prefix)
                    pred = render_predicate(head)
                    subject, rhs = pred.split(": ", 1)
                    if "<" in rhs:
                        cls, args = rhs.split("<", 1)
                        args = args[:-1]
                        app = f"{cls}<{subject}, {args}>"
                    else:
                        app = f"{rhs}<{subject}>"
                    replacement = default + "impl"
                    if vars_:
                        replacement += "<" + ", ".join(vars_) + ">"
                    replacement += " " + app
                if pred_ts:
                    replacement += " where " + render_predicates(pred_ts)
                replacement += " "
                edits.append((ts[start].start, ts[j].start, replacement))
                occupied.append((start, j))
                continue

            # Function/constructor/fallback header.
            name_i = i + 1 if tok.text == "function" else i
            paren_i = name_i + 1
            if paren_i >= len(ts) or ts[paren_i].text != "(":
                raise ValueError("function parameter list not found")
            end_paren = forward.get(paren_i)
            if end_paren is None:
                raise ValueError("unbalanced function parameters")
            j = end_paren + 1
            while j < len(ts) and ts[j].text not in {"{", ";"}:
                j += 1
            if j == len(ts):
                raise ValueError("unterminated function header")
            suffix = ts[end_paren + 1:j]
            arrow = top_index(suffix, {"->"})
            if arrow is not None:
                ret_ts = suffix[arrow + 1:]
                suffix_mods = [item.text for item in suffix[:arrow] if item.text in {"public", "payable"}]
            else:
                ret_ts = []
                suffix_mods = [item.text for item in suffix if item.text in {"public", "payable"}]
            modifiers.extend(mod for mod in suffix_mods if mod not in modifiers)
            if tok.text == "function":
                replacement = "function " + ts[name_i].text
                if vars_:
                    replacement += "<" + ", ".join(vars_) + ">"
            else:
                replacement = tok.text
            replacement += "(" + parse_params(ts[paren_i + 1:end_paren]) + ")"
            if modifiers:
                replacement += " " + " ".join(dict.fromkeys(modifiers))
            if ret_ts:
                ret = render_type(ret_ts)
                if ret != "()":
                    if ret.startswith("(") and ret.endswith(")"):
                        replacement += " returns " + ret
                    else:
                        replacement += " returns (" + ret + ")"
            if pred_ts:
                replacement += " where " + render_predicates(pred_ts)
            replacement += " "
            edits.append((ts[start].start, ts[j].start, replacement))
            occupied.append((start, j))
        except ValueError as error:
            warnings.append(f"cannot migrate {tok.text} declaration: {error}")
    return apply_edits(source, edits)


def transform_type_contexts(source: str, warnings: list[str]) -> str:
    ts = tokens(source)
    asm = assembly_ranges(ts)
    forward, _ = pairs(ts)
    edits = []

    # Typed local bindings.
    for i, tok in enumerate(ts):
        if tok.text != "let" or in_ranges(i, asm) or i + 2 >= len(ts) or ts[i + 2].text != ":":
            continue
        j = i + 3
        depth = 0
        while j < len(ts):
            if ts[j].text in "([<": depth += 1
            elif ts[j].text in ")]>" and depth: depth -= 1
            elif depth == 0 and ts[j].text in {"=", ";", ","}: break
            j += 1
        try:
            edits.append((ts[i + 3].start, ts[j - 1].end, render_type(ts[i + 3:j])))
        except (ValueError, IndexError) as error:
            warnings.append(f"cannot migrate let type: {error}")

    # Contract fields and type-alias RHS.  A field begins immediately after a
    # declaration boundary and consists of one identifier followed by a colon.
    for i, tok in enumerate(ts):
        if in_ranges(i, asm):
            continue
        if tok.text == "type":
            j = i + 1
            depth = 0
            while j < len(ts):
                if ts[j].text in "([<": depth += 1
                elif ts[j].text in ")]>" and depth: depth -= 1
                elif ts[j].text == "=" and depth == 0: break
                j += 1
            k = j + 1
            while k < len(ts) and ts[k].text != ";": k += 1
            if j < len(ts) and k < len(ts):
                try:
                    edits.append((ts[j + 1].start, ts[k - 1].end, render_type(ts[j + 1:k])))
                except ValueError as error:
                    warnings.append(f"cannot migrate type alias RHS: {error}")
        boundary = i == 0 or ts[i - 1].text in {"{", "}", ";"}
        if boundary and i + 1 < len(ts) and re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$-]*", tok.text) and ts[i + 1].text == ":":
            j = i + 2
            depth = 0
            while j < len(ts):
                if ts[j].text in "([<": depth += 1
                elif ts[j].text in ")]>" and depth: depth -= 1
                elif depth == 0 and ts[j].text in {"=", ";"}: break
                j += 1
            if j < len(ts):
                try:
                    edits.append((ts[i + 2].start, ts[j - 1].end, render_type(ts[i + 2:j])))
                except ValueError:
                    pass
    # Deduplicate exact edits (a malformed source can be recognized twice).
    edits = list(dict.fromkeys(edits))
    return apply_edits(source, edits)


def transform_signature_type_contexts(source: str, warnings: list[str]) -> str:
    """Canonicalize types in every callable signature, including arrowless ones."""
    ts = tokens(source)
    asm = assembly_ranges(ts)
    forward, _ = pairs(ts)
    edits = []
    for i, tok in enumerate(ts):
        if tok.text not in {"function", "constructor", "fallback", "lam"} or in_ranges(i, asm):
            continue
        j = i + 1
        while j < len(ts) and ts[j].text != "(" and ts[j].text not in {"{", ";"}:
            j += 1
        end = forward.get(j)
        if end is None:
            continue
        for param in split_top(ts[j + 1 : end]):
            colon = top_index(param, {":"})
            if colon is None or not param[colon + 1 :]:
                continue
            try:
                replacement = render_type(param[colon + 1 :])
            except ValueError as error:
                line = source.count("\n", 0, param[colon].start) + 1
                warnings.append(f"cannot migrate parameter type at line {line}: {error}")
                continue
            edits.append((param[colon + 1].start, param[-1].end, replacement))

        k = end + 1
        if tok.text == "lam" and k < len(ts) and ts[k].text == "->":
            type_start = k + 1
            type_end = type_start
            while type_end < len(ts) and ts[type_end].text != "{":
                type_end += 1
            if type_start < type_end:
                try:
                    edits.append(
                        (
                            ts[type_start].start,
                            ts[type_end - 1].end,
                            render_type(ts[type_start:type_end]),
                        )
                    )
                except ValueError as error:
                    line = source.count("\n", 0, ts[k].start) + 1
                    warnings.append(f"cannot migrate lambda result type at line {line}: {error}")
    return apply_edits(source, dict.fromkeys(edits))


def transform_proxy_annotations(source: str) -> str:
    ts = tokens(source)
    asm = assembly_ranges(ts)
    edits = []
    i = 0
    while i + 4 < len(ts):
        if in_ranges(i, asm) or ts[i].text != "Proxy" or ts[i + 1].text != ":" or ts[i + 2].text != "Proxy" or ts[i + 3].text not in {"(", "<"}:
            i += 1
            continue
        forward, _ = pairs(ts)
        end = forward.get(i + 3)
        if end is None:
            i += 1
            continue
        try:
            inner = render_type(ts[i + 4:end])
        except ValueError:
            i += 1
            continue
        edits.append((ts[i].start, ts[end].end, "@" + inner))
        i = end + 1
    return apply_edits(source, edits)


def delimiter_keys(ts: Sequence[Tok]) -> list[tuple[str, ...]]:
    """Return the delimiter stack immediately before each token."""
    keys: list[tuple[str, ...]] = []
    stack: list[str] = []
    matching = {")": "(", "]": "[", "}": "{"}
    for tok in ts:
        keys.append(tuple(stack))
        if tok.text in {"(", "[", "{"}:
            stack.append(tok.text)
        elif tok.text in matching and stack and stack[-1] == matching[tok.text]:
            stack.pop()
    return keys


def transform_conditionals(source: str, warnings: list[str]) -> str:
    """Canonicalize old conditional expressions and statement conditions.

    Expression `if c then x else y` is parenthesized when rewritten so nested
    conditionals keep the old tree even though canonical `?:` associates to
    the right.  Statement `if c { ... }` merely gains condition parentheses.
    """
    ts = tokens(source)
    asm = assembly_ranges(ts)
    keys = delimiter_keys(ts)

    # An `if` is an expression exactly when a same-delimiter `then` occurs
    # before its statement body or expression boundary.  Nested old `if`s are
    # deliberately not skipped: seeing either one's `then` is enough to
    # classify the outer token as expression syntax.
    expr_ifs: set[int] = set()
    for i, tok in enumerate(ts):
        if tok.text != "if" or in_ranges(i, asm):
            continue
        key = keys[i]
        for j in range(i + 1, len(ts)):
            if keys[j] != key:
                continue
            if ts[j].text == "then":
                expr_ifs.add(i)
                break
            if ts[j].text in {"{", ";", "}", "=>"}:
                break

    # Match if/then/else at each delimiter nesting.  A completed inner
    # conditional ends immediately before an enclosing then/else.  At an
    # ordinary expression boundary, all open else branches share that end.
    frames: dict[tuple[str, ...], list[dict[str, int | str | None]]] = {}
    complete: list[dict[str, int | str | None]] = []

    def close_else_frames(key: tuple[str, ...], end: int) -> None:
        stack = frames.setdefault(key, [])
        while stack and stack[-1]["state"] == "else":
            frame = stack.pop()
            frame["end"] = end
            complete.append(frame)

    for i, tok in enumerate(ts):
        if in_ranges(i, asm):
            continue
        key = keys[i]
        text = tok.text
        if text == "if" and i in expr_ifs:
            frames.setdefault(key, []).append(
                {"if": i, "then": None, "else": None, "end": None, "state": "cond"}
            )
            continue
        if text == "then":
            close_else_frames(key, tok.start)
            stack = frames.setdefault(key, [])
            if stack and stack[-1]["state"] == "cond":
                stack[-1]["then"] = i
                stack[-1]["state"] = "then"
            continue
        if text == "else":
            close_else_frames(key, tok.start)
            stack = frames.setdefault(key, [])
            if stack and stack[-1]["state"] == "then":
                stack[-1]["else"] = i
                stack[-1]["state"] = "else"
            continue
        if text in {",", ";", "=>"}:
            close_else_frames(key, tok.start)
        elif text in {")", "]", "}"}:
            # The key before a closing delimiter still contains that
            # delimiter; conditionals inside it end at the close token.
            close_else_frames(key, tok.start)

    for stack in frames.values():
        while stack and stack[-1]["state"] == "else":
            frame = stack.pop()
            frame["end"] = len(source)
            complete.append(frame)
        for frame in stack:
            line = source.count("\n", 0, ts[int(frame["if"])].start) + 1
            warnings.append(f"cannot migrate incomplete conditional expression at line {line}")

    edits: list[tuple[int, int, str]] = []
    boundary_by_start = {tok.start: tok.text for tok in ts}
    expression_starts = {
        "(", "[", "{", ",", ";", "=", ":", "?", "=>",
        "return", "then", "else",
    }
    for frame in complete:
        if_i = int(frame["if"])
        then_i = int(frame["then"])
        else_i = int(frame["else"])
        end = int(frame["end"])
        previous = ts[if_i - 1].text if if_i else None
        # Canonical ternaries already associate correctly through their then
        # and else branches.  Parentheses are needed when this conditional is
        # itself another conditional's condition, or appears as an operand of
        # a stronger operator.  Avoiding redundant parentheses also preserves
        # the dedicated excessive-conditional-nesting diagnostic instead of
        # turning it into delimiter nesting.
        need_parens = (
            boundary_by_start.get(end) == "then"
            or (previous is not None and previous not in expression_starts)
        )
        edits.extend(
            [
                (ts[if_i].start, ts[if_i].end, "(" if need_parens else ""),
                (ts[then_i].start, ts[then_i].end, " ? "),
                (ts[else_i].start, ts[else_i].end, " : "),
            ]
        )
        if need_parens:
            edits.append((end, end, ")"))

    # Statement conditions: find their same-delimiter opening body brace and
    # wrap only when the condition is not already one outer parenthesized
    # expression.
    forward, _ = pairs(ts)
    for i, tok in enumerate(ts):
        if tok.text != "if" or i in expr_ifs or in_ranges(i, asm):
            continue
        key = keys[i]
        brace = None
        for j in range(i + 1, len(ts)):
            if keys[j] == key and ts[j].text == "{":
                brace = j
                break
            if keys[j] == key and ts[j].text in {";", "}", "=>"}:
                break
        if brace is None or brace == i + 1:
            continue
        already_parenthesized = (
            ts[i + 1].text == "("
            and forward.get(i + 1) == brace - 1
        )
        if not already_parenthesized:
            edits.append((tok.end, tok.end, " ("))
            edits.append((ts[brace].start, ts[brace].start, ") "))

    return apply_edits(source, edits)


def transform_core_colon_equals(source: str) -> str:
    ts = tokens(source)
    asm = assembly_ranges(ts)
    edits = [
        (tok.start, tok.end, "=")
        for i, tok in enumerate(ts)
        if tok.text == ":=" and not in_ranges(i, asm)
    ]
    return apply_edits(source, edits)


def transform_expression_annotations(source: str, warnings: list[str]) -> str:
    """Remove legacy expected-type annotations from expressions.

    The canonical syntax gets expected types from the surrounding return,
    argument, assignment, or typed-binding context.  Declaration/predicate
    colons and ternary separators are protected explicitly; every remaining
    colon which is followed by an old type is a legacy expression annotation.
    """
    ts = tokens(source)
    asm = assembly_ranges(ts)
    forward, _ = pairs(ts)
    protected: set[int] = set()

    # Named parameter colons for functions, constructors, fallbacks, and
    # lambdas.  Only top-level entries in the parameter list bind names.
    for i, tok in enumerate(ts):
        if tok.text not in {"function", "constructor", "fallback", "lam"} or in_ranges(i, asm):
            continue
        j = i + 1
        while j < len(ts) and ts[j].text != "(" and ts[j].text not in {"{", ";"}:
            j += 1
        end = forward.get(j)
        if end is None:
            continue
        depth = 0
        for k in range(j + 1, end):
            if ts[k].text in "([":
                depth += 1
            elif ts[k].text in ")]" and depth:
                depth -= 1
            elif ts[k].text == ":" and depth == 0:
                protected.add(k)

    # Typed locals.
    for i, tok in enumerate(ts[:-2]):
        if tok.text == "let" and ts[i + 2].text == ":" and not in_ranges(i, asm):
            protected.add(i + 2)

    # Where-clause predicates.  The clause terminates at the declaration body
    # or method-signature semicolon.
    for i, tok in enumerate(ts):
        if tok.text != "where" or in_ranges(i, asm):
            continue
        j = i + 1
        depth = 0
        while j < len(ts):
            if ts[j].text in "([":
                depth += 1
            elif ts[j].text in ")]" and depth:
                depth -= 1
            elif depth == 0 and ts[j].text in {"{", ";"}:
                break
            elif ts[j].text == ":":
                protected.add(j)
            j += 1

    # Protect only direct contract members.  A bare `expr : T;` at the start
    # of a function statement is an annotation, not a field.
    for i, tok in enumerate(ts):
        if tok.text != "contract" or in_ranges(i, asm):
            continue
        opening = i + 1
        while opening < len(ts) and ts[opening].text != "{":
            opening += 1
        closing = forward.get(opening)
        if closing is None:
            continue
        depth = 0
        for j in range(opening + 1, closing - 1):
            if ts[j].text == "{":
                depth += 1
                continue
            if ts[j].text == "}" and depth:
                depth -= 1
                continue
            boundary = j == opening + 1 or ts[j - 1].text in {"}", ";"}
            if (
                depth == 0
                and boundary
                and re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$-]*", ts[j].text)
                and ts[j + 1].text == ":"
            ):
                protected.add(j + 1)

    # Pair each ternary question mark with its same-nesting colon.  A stack is
    # needed for right-nested conditional expressions.
    delimiter_stack: list[str] = []
    questions: dict[tuple[str, ...], list[int]] = {}
    matching = {")": "(", "]": "[", "}": "{"}
    for i, tok in enumerate(ts):
        if in_ranges(i, asm):
            continue
        if tok.text in "([{":
            delimiter_stack.append(tok.text)
            continue
        if tok.text in matching:
            if delimiter_stack and delimiter_stack[-1] == matching[tok.text]:
                delimiter_stack.pop()
            continue
        key = tuple(delimiter_stack)
        if tok.text == "?":
            questions.setdefault(key, []).append(i)
        elif tok.text == ":" and questions.get(key):
            questions[key].pop()
            protected.add(i)

    edits = []
    for i, tok in enumerate(ts):
        if tok.text != ":" or i in protected or in_ranges(i, asm):
            continue
        parser = TypeParser(ts[i + 1 :])
        try:
            parser.parse()
        except ValueError as error:
            line = source.count("\n", 0, tok.start) + 1
            warnings.append(f"cannot remove expression annotation at line {line}: {error}")
            continue
        if parser.i == 0:
            continue
        end = ts[i + parser.i]
        edits.append((tok.start, end.end, ""))
    return apply_edits(source, edits)


def transform_matches(source: str, warnings: list[str]) -> str:
    # Match bodies need nested statement-aware brace insertion.  Process the
    # innermost match first and retokenize after every replacement.
    while True:
        ts = tokens(source)
        asm = assembly_ranges(ts)
        forward, _ = pairs(ts)
        candidate = None
        for i, tok in enumerate(ts):
            if tok.text != "match" or in_ranges(i, asm):
                continue
            j = i + 1
            if j < len(ts) and ts[j].text == "(" and j in forward:
                after = forward[j] + 1
                if after < len(ts) and ts[after].text == "{":
                    j = after
                else:
                    j = i + 1
            if j == i + 1:
                depth = 0
                while j < len(ts):
                    if ts[j].text in "([": depth += 1
                    elif ts[j].text in ")]" and depth: depth -= 1
                    elif ts[j].text == "{" and depth == 0: break
                    j += 1
            if j == len(ts) or j not in forward:
                continue
            match_body = ts[j + 1 : forward[j]]
            # Canonical bodies begin with case/default.  Parentheses around an
            # old scrutinee do not make a pipe-arm body canonical.
            if not legacy_match_arm_starts(match_body):
                continue
            candidate = (i, j, forward[j])
        if candidate is None:
            return source
        i, brace, end = candidate
        body = ts[brace + 1:end]
        arms = []
        max_pattern_arity = 1
        starts = legacy_match_arm_starts(body)
        if not starts:
            warnings.append("match has no recognizable legacy arms")
            return source
        for n, start in enumerate(starts):
            stop = starts[n + 1] if n + 1 < len(starts) else len(body)
            arm = body[start + 1:stop]
            fat = top_index(arm, {"=>"}, angles=False)
            if fat is None:
                warnings.append("match arm has no =>")
                return source
            pats = arm[:fat]
            pat_parts = [part for part in split_top(pats, angles=False) if part]
            max_pattern_arity = max(max_pattern_arity, len(pat_parts))
            wildcard = bool(pat_parts) and all(
                len(part) == 1 and part[0].text == "_" for part in pat_parts
            )
            if wildcard:
                head = "default"
            else:
                pat = source[body[start].end : arm[fat].start].strip()
                if len(pat_parts) > 1:
                    pat = "(" + pat + ")"
                head = "case " + pat
            body_end = body[stop].start if stop < len(body) else ts[end].start
            stmts = source[arm[fat].end : body_end].strip()
            arms.append(head + " {\n" + stmts + "\n}")
        scrutinee_ts = ts[i + 1:brace]
        if len(scrutinee_ts) >= 2 and scrutinee_ts[0].text == "(" and scrutinee_ts[-1].text == ")":
            local_forward, _ = pairs(scrutinee_ts)
            if local_forward.get(0) == len(scrutinee_ts) - 1:
                inner = scrutinee_ts[1:-1]
                tuple_scrutinee = len(
                    [part for part in split_top(inner, angles=False) if part]
                ) > 1
                # Legacy `match (a, b) { | x => ... }` has one tuple
                # scrutinee. Canonical match parentheses delimit a scrutinee
                # list, so retain the expression parentheses as a nested pair.
                # Multi-pattern arms (`| p, q =>`) identify the genuinely
                # multi-scrutinee form and may drop the legacy wrapper.
                if not tuple_scrutinee or max_pattern_arity > 1:
                    scrutinee_ts = inner
        scrutinees = source_slice(source, scrutinee_ts).strip()
        leading = source[ts[brace].end : body[starts[0]].start].strip()
        contents = ((leading + "\n") if leading else "") + "\n".join(arms)
        replacement = "match (" + scrutinees + ") {\n" + contents + "\n}"
        replace_end = ts[end + 1].end if end + 1 < len(ts) and ts[end + 1].text == ";" else ts[end].end
        source = apply_edits(source, [(ts[i].start, replace_end, replacement)])


def legacy_match_arm_starts(body: Sequence[Tok]) -> list[int]:
    """Return top-level pipes which introduce a legacy `| pat =>` arm.

    A top-level bitwise-or in an arm body is not an arm separator.  A candidate
    pipe counts only when a top-level fat arrow occurs before the next
    top-level pipe.
    """
    pipes: list[int] = []
    depth = 0
    for i, tok in enumerate(body):
        if tok.text in "({[":
            depth += 1
        elif tok.text in ")}]" and depth:
            depth -= 1
        elif tok.text == "|" and depth == 0:
            pipes.append(i)
    starts = []
    for position, start in enumerate(pipes):
        stop = pipes[position + 1] if position + 1 < len(pipes) else len(body)
        depth = 0
        for tok in body[start + 1 : stop]:
            if tok.text in "({[":
                depth += 1
            elif tok.text in ")}]" and depth:
                depth -= 1
            elif tok.text == "=>" and depth == 0:
                starts.append(start)
                break
    return starts


def tracked_targets() -> list[pathlib.Path]:
    output = subprocess.check_output(
        ["git", "ls-files", "--", "*.solc"], cwd=ROOT, text=True
    )
    targets = []
    for relative in output.splitlines():
        if not (
            relative.startswith("std/")
            or relative.startswith("tests/e2e/")
            or relative.startswith("fuzz/corpus/")
            or re.match(r"^crates/[^/]+/tests/fixtures/", relative)
        ):
            continue
        path = ROOT / relative
        if not path.exists():
            path = path.with_suffix(".sol")
        if path.exists():
            targets.append(path)
    return targets


def unsafe_paths() -> set[str]:
    unsafe = set()
    manifest = ROOT / "crates/parser/tests/fixtures/corpus/reference-frontend.tsv"
    if manifest.exists():
        for line in manifest.read_text().splitlines():
            cols = line.split("\t")
            if len(cols) >= 3 and cols[1:3] == ["fail", "SC0001"]:
                name = pathlib.Path(cols[0]).with_suffix(".solc").as_posix()
                unsafe.add("crates/parser/tests/fixtures/corpus/fail/test/examples/" + name)
    unsafe.update(
        {
            "crates/parser/tests/fixtures/corpus/fail/test/diagnostics/parse-error.solc",
            "crates/parser/tests/fixtures/corpus/fail/test/imports/select_alias_tail_fail.solc",
        }
    )
    for path in subprocess.check_output(
        ["git", "ls-files", "--", "crates/uitest/tests/fixtures/parse/*.solc", "crates/uitest/tests/fixtures/parse/**/*.solc"],
        cwd=ROOT,
        text=True,
    ).splitlines():
        unsafe.add(path)
    return unsafe


def old_relative(path: pathlib.Path) -> str:
    relative = path.relative_to(ROOT).as_posix()
    return relative[:-4] + ".solc" if relative.endswith(".sol") else relative


def migrate(source: str, warnings: list[str]) -> str:
    source = transform_imports(source, warnings)
    source = transform_data(source, warnings)
    source = transform_declarations(source, warnings)
    source = transform_signature_type_contexts(source, warnings)
    source = transform_type_contexts(source, warnings)
    source = transform_proxy_annotations(source)
    source = transform_matches(source, warnings)
    source = transform_conditionals(source, warnings)
    source = transform_core_colon_equals(source)
    source = transform_expression_annotations(source, warnings)
    return source


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true", help="report changes without writing")
    parser.add_argument("paths", nargs="*", help="optional repository-relative source paths")
    args = parser.parse_args()

    targets = [ROOT / path for path in args.paths] if args.paths else tracked_targets()
    unsafe = unsafe_paths()
    changed = 0
    report = [
        "# Syntax migration exceptions",
        "",
        "The following tracked sources required a conservative migration or",
        "still contain an intentionally malformed construct.  Intentional",
        "parser-error fixtures are not skipped wholesale: independent syntax",
        "around the error is rewritten normally.",
        "Paths use their pre-extension-migration `.solc` spelling.",
        "",
    ]
    for path in targets:
        relative = old_relative(path)
        intentional_error = relative in unsafe
        original = path.read_text()
        warnings: list[str] = []
        try:
            migrated = migrate(original, warnings)
        except (ValueError, IndexError) as error:
            migrated = original
            warnings.append(f"file-level migration failure: {error}")
        if warnings:
            for warning in sorted(set(warnings)):
                fixture = "intentional parser-error fixture; " if intentional_error else ""
                report.append(f"- `{relative}` — {fixture}{warning}")
        if migrated != original:
            changed += 1
            if not args.check:
                path.write_text(migrated)

    report_text = "\n".join(report) + "\n"
    if not args.check:
        REPORT.write_text(report_text)
    print(f"{len(targets)} source files inspected; {changed} would change" if args.check else f"{len(targets)} source files inspected; {changed} changed")
    if args.check and changed:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
