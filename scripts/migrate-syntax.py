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


