#!/usr/bin/env python3
"""Migrate legacy Solcore syntax inside Rust, TypeScript, and JavaScript strings.

Only literals containing recognizable Solcore declarations are rewritten.
Rust format templates are decoded with interpolation placeholders protected,
then their literal source braces are escaped again after migration.  This is a
temporary migration aid for the syntax cut-over.
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import sys
from pathlib import Path


IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
PRESERVE_NEXT_LITERAL_MARKER = "syntax-migration: preserve-next-literal"
PRESERVE_LITERALS_BEGIN_MARKER = "syntax-migration: preserve-literals-begin"
PRESERVE_LITERALS_END_MARKER = "syntax-migration: preserve-literals-end"


def load_source_migrator():
    path = Path(__file__).with_name("migrate-syntax.py")
    if not path.is_file():
        return None
    spec = importlib.util.spec_from_file_location("solcore_source_migrator", path)
    if spec is None or spec.loader is None:
        return None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


SOURCE_MIGRATOR = load_source_migrator()
SOURCE_MARKER = re.compile(
    r"(?m)^\s*(?:(?:import|export|pragma)\b[^\n;]*;|type\s+[A-Za-z_][A-Za-z0-9_]*\s*=|"
    r"data\s+[A-Za-z_][A-Za-z0-9_.,:()<> \t-]*(?:=|;)|"
    r"(?:enum|class|trait|instance)\s+[A-Za-z_(][A-Za-z0-9_.,:()<> \t-]*\{|"
    r"impl(?:\s*<[^{}\n>]*>)?\s+[A-Za-z_(][A-Za-z0-9_.,:()<> \t-]*\{|"
    r"default\s+(?:instance\s+|impl(?:\s*<[^{}\n>]*>)?\s+)[A-Za-z_(][A-Za-z0-9_.,:()<> \t-]*\{|"
    r"function\s+[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^\n>]+>)?\s*\(|"
    r"contract\s+[A-Za-z_][A-Za-z0-9_]*\s*\{|(?:constructor|fallback)\s*\(|"
    r"(?:public|payable)(?:\s+(?:public|payable))*\s+function\s+"
    r"[A-Za-z_][A-Za-z0-9_]*\s*(?:<[^\n>]+>)?\s*\()"
)
MATCH_SOURCE_MARKER = re.compile(
    r"(?m)^\s*match\s*(?:\([^\n{}]*\)|[A-Za-z_][A-Za-z0-9_.]*)\s*\{\s*"
    r"(?:\||case\b|default\b|})"
)


def matching_paren(text: str, start: int) -> int | None:
    pairs = {"(": ")", "[": "]", "<": ">"}
    opening = text[start]
    closing = pairs[opening]
    depth = 0
    for index in range(start, len(text)):
        char = text[index]
        if char == opening:
            depth += 1
        elif char == closing and not (
            opening == "<" and index > start and text[index - 1] in "=-"
        ):
            depth -= 1
            if depth == 0:
                return index
    return None


def split_top_level(text: str, delimiter: str = ",") -> list[str]:
    parts: list[str] = []
    start = 0
    paren = bracket = angle = 0
    for index, char in enumerate(text):
        if char == "(":
            paren += 1
        elif char == ")":
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]":
            bracket -= 1
        elif char == "<":
            angle += 1
        elif char == ">" and angle and (index == 0 or text[index - 1] not in "=-"):
            angle -= 1
        elif char == delimiter and paren == bracket == angle == 0:
            parts.append(text[start:index])
            start = index + 1
    parts.append(text[start:])
    return parts


def top_level_arrow(text: str) -> int | None:
    paren = bracket = angle = 0
    index = 0
    while index + 1 < len(text):
        char = text[index]
        if char == "(":
            paren += 1
        elif char == ")":
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]":
            bracket -= 1
        elif char == "<":
            angle += 1
        elif text[index : index + 2] == "->" and paren == bracket == angle == 0:
            return index
        elif char == ">" and angle and (index == 0 or text[index - 1] not in "=-"):
            angle -= 1
        index += 1
    return None


def convert_type(text: str) -> str:
    leading = text[: len(text) - len(text.lstrip())]
    trailing = text[len(text.rstrip()) :]
    core = text.strip()
    if not core:
        return text

    if core == "()":
        return leading + core + trailing

    canonical_fn = re.fullmatch(
        r"function\s*\((?P<params>.*)\)\s*returns\s*\((?P<ret>.*)\)",
        core,
        re.S,
    )
    if canonical_fn:
        params = ",".join(convert_type(part) for part in split_top_level(canonical_fn.group("params")))
        ret = convert_type(canonical_fn.group("ret"))
        return f"{leading}function({params}) returns({ret}){trailing}"

    arrow = top_level_arrow(core)
    if arrow is not None:
        lhs = convert_type(core[:arrow]).strip()
        rhs = convert_type(core[arrow + 2 :])
        # Parentheses here describe the function parameter list.  Preserve a
        # doubly-parenthesized tuple parameter as one tuple parameter.
        if lhs == "()":
            params = ""
        elif lhs.startswith("(") and lhs.endswith(")"):
            params = lhs[1:-1]
        else:
            params = lhs
        return f"{leading}function({params}) returns({rhs.strip()}){trailing}"

    out: list[str] = []
    index = 0
    while index < len(core):
        comptime = re.match(r"comptime\s+", core[index:])
        if comptime:
            atom_start = index + comptime.end()
            atom_match = IDENT.match(core, atom_start)
            if atom_match:
                atom_end = atom_match.end()
                while atom_end < len(core) and core[atom_end] == ".":
                    segment = IDENT.match(core, atom_end + 1)
                    if not segment:
                        break
                    atom_end = segment.end()
                if atom_end < len(core) and core[atom_end] == "(":
                    close = matching_paren(core, atom_end)
                    if close is not None:
                        atom_end = close + 1
                atom = convert_type(core[atom_start:atom_end])
                out.append(f"comptime<{atom}>")
                index = atom_end
                continue

        match = IDENT.match(core, index)
        if match:
            name_end = match.end()
            while name_end < len(core) and core[name_end] == ".":
                segment = IDENT.match(core, name_end + 1)
                if not segment:
                    break
                name_end = segment.end()
            name = core[index:name_end]
            if name_end < len(core) and core[name_end] in "(<":
                opener = core[name_end]
                close = matching_paren(core, name_end)
                if close is not None:
                    inner = core[name_end + 1 : close]
                    converted = [convert_type(part) for part in split_top_level(inner)]
                    mapping_arrow = top_level_fat_arrow(inner)
                    if name == "mapping" and mapping_arrow is not None:
                        key = convert_type(inner[:mapping_arrow]).strip()
                        value = convert_type(inner[mapping_arrow + 2 :]).strip()
                        out.append(f"mapping({key} => {value})")
                    elif name == "mapping" and len(converted) == 2:
                        out.append(f"mapping({converted[0]} => {converted[1]})")
                    elif name == "function":
                        out.append(f"function({','.join(converted)})")
                    elif name == "returns":
                        out.append(f"returns ({','.join(converted)})")
                    else:
                        out.append(f"{name}<{','.join(converted)}>")
                    index = close + 1
                    continue
            out.append(name)
            index = name_end
            continue

        if core[index] == "(":
            close = matching_paren(core, index)
            if close is not None:
                inner = core[index + 1 : close]
                out.append("(" + ",".join(convert_type(part) for part in split_top_level(inner)) + ")")
                index = close + 1
                continue
        out.append(core[index])
        index += 1
    return leading + "".join(out) + trailing


def top_level_fat_arrow(text: str) -> int | None:
    paren = bracket = angle = 0
    index = 0
    while index + 1 < len(text):
        char = text[index]
        if char == "(":
            paren += 1
        elif char == ")":
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]":
            bracket -= 1
        elif text[index : index + 2] == "=>" and paren == bracket == angle == 0:
            return index
        elif char == "<":
            angle += 1
        elif char == ">" and angle and (index == 0 or text[index - 1] not in "=-"):
            angle -= 1
        index += 1
    return None


def find_top_level_colon(text: str) -> int | None:
    paren = bracket = angle = 0
    for index, char in enumerate(text):
        if char == "(":
            paren += 1
        elif char == ")":
            paren -= 1
        elif char == "[":
            bracket += 1
        elif char == "]":
            bracket -= 1
        elif char == "<":
            angle += 1
        elif char == ">" and angle and (index == 0 or text[index - 1] not in "=-"):
            angle -= 1
        elif char == ":" and paren == bracket == angle == 0:
            return index
    return None


def convert_predicate(text: str) -> str:
    stripped = text.strip()
    colon = find_top_level_colon(stripped)
    if colon is None:
        return stripped
    subject = convert_type(stripped[:colon])
    class_ref = stripped[colon + 1 :].strip()
    match = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_.]*)(?:\((.*)\))?", class_ref, re.S)
    if match and match.group(2) is not None:
        args = ",".join(convert_type(part) for part in split_top_level(match.group(2)))
        class_ref = f"{match.group(1)}<{args}>"
    return f"{subject}: {class_ref}"


def convert_predicates(text: str) -> str:
    return ", ".join(convert_predicate(part) for part in split_top_level(text))


def convert_params(text: str) -> str:
    converted: list[str] = []
    for part in split_top_level(text):
        colon = find_top_level_colon(part)
        if colon is None:
            converted.append(part)
        else:
            converted.append(part[: colon + 1] + convert_type(part[colon + 1 :]))
    return ",".join(converted)


def convert_imports(source: str) -> str:
    pattern = re.compile(
        r"\bimport\s+(@?[A-Za-z_][A-Za-z0-9_.]*)\.\{([^{}]+)\}\s*;"
    )

    def selected(match: re.Match[str]) -> str:
        path, names = match.groups()
        names = names.strip()
        if names == "*":
            return f"import * from {path};"
        return f"import {{{names}}} from {path};"

    source = pattern.sub(selected, source)
    source = re.sub(
        r"\bimport\s+(@?[A-Za-z_][A-Za-z0-9_.]*)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;",
        r"import * as \2 from \1;",
        source,
    )
    return source


DATA = re.compile(
    r"^(?P<indent>[ \t]*)data\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    r"(?:\((?P<params>[^()\n]*)\))?\s*(?:=\s*(?P<body>.*?))?;(?P<tail>[ \t]*)$",
    re.M | re.S,
)


def convert_ctor(ctor: str) -> str:
    match = re.fullmatch(r"(\s*[A-Za-z_][A-Za-z0-9_]*\s*)\((.*)\)(\s*)", ctor, re.S)
    if not match:
        return ctor
    fields = ",".join(convert_type(part) for part in split_top_level(match.group(2)))
    return f"{match.group(1)}({fields}){match.group(3)}"


def convert_data(source: str) -> str:
    def replacement(match: re.Match[str]) -> str:
        indent = match.group("indent")
        name = match.group("name")
        params = match.group("params")
        body = match.group("body")
        generic = "" if params is None else f"<{params}>"
        if body is None:
            return f"{indent}enum {name}{generic} {{}}{match.group('tail')}"
        ctors = split_top_level(body, "|")
        converted = ",".join(convert_ctor(ctor) for ctor in ctors)
        return f"{indent}enum {name}{generic} {{{converted}}}{match.group('tail')}"

    return DATA.sub(replacement, source)


def parse_legacy_class_header(line: str) -> str | None:
    match = re.match(
        r"^(?P<indent>\s*)(?:forall\s+(?P<vars>[^.\n]+)\s*\.\s*)?"
        r"(?:(?P<constraints>.*?)\s*=>\s*)?class\s+"
        r"(?P<subject>[A-Za-z_][A-Za-z0-9_]*)\s*:\s*"
        r"(?P<class>[A-Za-z_][A-Za-z0-9_]*)"
        r"(?:\((?P<args>.*)\))?\s*(?P<rest>\{.*)$",
        line,
    )
    if not match:
        return None
    params = [match.group("subject")]
    if match.group("args"):
        params.extend(part.strip() for part in split_top_level(match.group("args")))
    result = f"{match.group('indent')}trait {match.group('class')}<{','.join(params)}>"
    if match.group("constraints"):
        result += f" where {convert_predicates(match.group('constraints'))}"
    return result + " " + match.group("rest")


def split_impl_head(text: str) -> tuple[str, str, str | None] | None:
    colon = find_top_level_colon(text)
    if colon is None:
        return None
    subject = text[:colon].strip()
    class_ref = text[colon + 1 :].strip()
    match = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*)(?:\((.*)\))?", class_ref, re.S)
    if not match:
        return None
    return subject, match.group(1), match.group(2)


def parse_legacy_impl_header(line: str) -> str | None:
    match = re.match(
        r"^(?P<indent>\s*)(?:forall\s+(?P<vars>[^.\n]+)\s*\.\s*)?"
        r"(?:(?P<constraints>.*?)\s*=>\s*)?"
        r"(?P<default>default\s+)?instance\s+(?P<head>.*?)\s*(?P<rest>\{.*)$",
        line,
    )
    if not match:
        return None
    parsed = split_impl_head(match.group("head"))
    if parsed is None:
        return None
    subject, class_name, args = parsed
    head_args = [convert_type(subject)]
    if args is not None:
        head_args.extend(convert_type(part) for part in split_top_level(args))
    prefix = "default impl" if match.group("default") else "impl"
    vars_text = match.group("vars")
    generics = ""
    if vars_text:
        generics = "<" + ",".join(vars_text.replace(",", " ").split()) + ">"
    result = (
        f"{match.group('indent')}{prefix}{generics} {class_name}"
        f"<{','.join(head_args)}>"
    )
    if match.group("constraints"):
        result += f" where {convert_predicates(match.group('constraints'))}"
    return result + " " + match.group("rest")


FUNCTION_PREFIX = re.compile(
    r"^(?P<indent>\s*)"
    r"(?:(?P<forall>forall\s+(?P<vars>[^.\n]+)\s*\.\s*))?"
    r"(?:(?P<constraints>.*?)\s*=>\s*)?"
    r"(?P<prefix>(?:(?:public|payable)\s+)*)function\s+"
    r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)


def parse_legacy_function_header(line: str, inherited_vars: str | None = None) -> str | None:
    match = FUNCTION_PREFIX.match(line)
    if not match:
        return None
    cursor = match.end()
    if cursor >= len(line) or line[cursor] != "(":
        return None
    params_end = matching_paren(line, cursor)
    if params_end is None:
        return None
    params_text = line[cursor + 1 : params_end]
    tail = line[params_end + 1 :].lstrip()
    suffix_match = re.match(r"(?P<mods>(?:(?:public|payable)\s+)*)", tail)
    assert suffix_match is not None
    suffix = suffix_match.group("mods")
    tail = tail[suffix_match.end() :]
    ret: str | None = None
    if tail.startswith("->"):
        tail = tail[2:].lstrip()
        boundary = None
        paren = bracket = angle = 0
        for index, char in enumerate(tail):
            if char == "(":
                paren += 1
            elif char == ")":
                paren -= 1
            elif char == "[":
                bracket += 1
            elif char == "]":
                bracket -= 1
            elif char == "<":
                angle += 1
            elif char == ">" and angle:
                angle -= 1
            elif char in "{;" and paren == bracket == angle == 0:
                boundary = index
                break
        if boundary is None:
            return None
        ret = tail[:boundary].strip()
        end = tail[boundary:]
    elif tail.startswith(("{", ";")):
        end = tail
    else:
        return None
    # Already-canonical headers do not need reconstruction.
    if not match.group("forall") and ret is None and not match.group("prefix"):
        return None
    params = "(" + convert_params(params_text) + ")"
    vars_text = match.group("vars") or inherited_vars
    generics = ""
    if vars_text:
        generics = "<" + ",".join(vars_text.replace(",", " ").split()) + ">"
    modifiers = (match.group("prefix") + suffix).split()
    modifier_text = "" if not modifiers else " " + " ".join(dict.fromkeys(modifiers))
    result = f"{match.group('indent')}function {match.group('name')}{generics}{params}{modifier_text}"
    if ret is not None:
        converted_ret = convert_type(ret)
        result += " returns " + ("()" if converted_ret.strip() == "()" else f"({converted_ret})")
    if match.group("constraints"):
        result += f" where {convert_predicates(match.group('constraints'))}"
    return result + " " + end


def brace_delta(line: str) -> int:
    # Good enough for source fixtures: braces in comments/string literals do
    # not occur on assembly boundary lines in the migrated corpus.
    return line.count("{") - line.count("}")


def convert_headers(source: str) -> str:
    # Legacy `forall ... . constraints =>` prefixes may span one or more
    # lines.  Coalesce just those prefixes so the structural header parsers can
    # see the complete declaration, then restore the declaration indentation.
    source = re.sub(
        r"(?m)^(?P<indent>[ \t]*)forall\s+(?P<vars>[^.\n]+)\s*\.\s*\n"
        r"(?:(?P<constraints>[^\n]+?)\s*=>\s*\n)?"
        r"(?P<header>[ \t]*(?:function|class|instance|default\s+instance)\b)",
        lambda m: (
            f"{m.group('indent')}forall {m.group('vars')} . "
            + (f"{m.group('constraints').strip()} => " if m.group("constraints") else "")
            + m.group("header").lstrip()
        ),
        source,
    )
    source = re.sub(
        r"(?m)^(?P<indent>[ \t]*)forall\s+(?P<vars>[^.\n]+)\s*\.\s*"
        r"(?P<constraints>[^\n=]+?)\s*=>\s*\n"
        r"(?P<header>[ \t]*(?:function|class|instance|default\s+instance)\b)",
        lambda m: (
            f"{m.group('indent')}forall {m.group('vars')} . "
            f"{m.group('constraints').strip()} => {m.group('header').lstrip()}"
        ),
        source,
    )
    lines = source.splitlines(keepends=True)
    output: list[str] = []
    assembly_depth = 0
    pending_forall: str | None = None
    for original in lines:
        newline = "\n" if original.endswith("\n") else ""
        line = original[:-1] if newline else original
        in_assembly = assembly_depth > 0
        if not in_assembly:
            forall_only = re.match(r"^\s*forall\s+([^.\n]+)\s*\.\s*$", line)
            if forall_only:
                pending_forall = forall_only.group(1)
                continue
            converted = (
                parse_legacy_class_header(line)
                or parse_legacy_impl_header(line)
                or parse_legacy_function_header(line, pending_forall)
            )
            if converted is not None:
                line = converted
                pending_forall = None
            elif line.strip() and not line.strip().startswith("//"):
                pending_forall = None
        if not in_assembly and re.search(r"\bassembly(?:\s*\([^)]*\))?\s*\{", line):
            assembly_depth = max(0, brace_delta(line))
        elif in_assembly:
            assembly_depth = max(0, assembly_depth + brace_delta(line))
        output.append(line + newline)
    return "".join(output)


def convert_signature_types(source: str) -> str:
    # A second pass over canonicalized function headers converts every
    # parameter/return type, including headers whose return was `()` and thus
    # looked canonical enough to the legacy-header pass.
    lines = source.splitlines(keepends=True)
    output: list[str] = []
    assembly_depth = 0
    for original in lines:
        newline = "\n" if original.endswith("\n") else ""
        line = original[:-1] if newline else original
        if assembly_depth == 0:
            match = re.match(
                r"^(?P<indent>\s*)function\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
                r"(?P<generics><[^>]+>)?",
                line,
            )
            if match:
                params_start = match.end()
                if params_start < len(line) and line[params_start] == "(":
                    params_end = matching_paren(line, params_start)
                    if params_end is not None:
                        params = convert_params(line[params_start + 1 : params_end])
                        tail = line[params_end + 1 :]
                        returns = re.search(r"\breturns\s*\(", tail)
                        if returns:
                            ret_start = tail.find("(", returns.start())
                            ret_end = matching_paren(tail, ret_start)
                            if ret_end is not None:
                                ret_parts = split_top_level(tail[ret_start + 1 : ret_end])
                                ret = ",".join(convert_type(part) for part in ret_parts)
                                tail = tail[: ret_start + 1] + ret + tail[ret_end:]
                        line = (
                            f"{match.group('indent')}function {match.group('name')}"
                            f"{match.group('generics') or ''}({params}){tail}"
                        )
        if assembly_depth == 0 and re.search(r"\bassembly(?:\s*\([^)]*\))?\s*\{", line):
            assembly_depth = max(0, brace_delta(line))
        elif assembly_depth:
            assembly_depth = max(0, assembly_depth + brace_delta(line))
        output.append(line + newline)
    return "".join(output)


def convert_match_body(body: str) -> str:
    # Legacy arms start at `|` and are separated by the next top-level `|` or
    # the match body's closing brace.  Arm bodies are statements rather than
    # expressions, so they may be wrapped in braces without semantic change.
    out: list[str] = []
    cursor = 0
    while cursor < len(body):
        arm = re.search(r"(?m)^(?P<indent>[ \t]*)\|\s*", body[cursor:])
        if arm is None:
            out.append(body[cursor:])
            break
        start = cursor + arm.start()
        marker_end = cursor + arm.end()
        out.append(body[cursor:start])
        arrow = body.find("=>", marker_end)
        if arrow < 0:
            out.append(body[start:])
            break
        pattern = body[marker_end:arrow].strip()
        body_start = arrow + 2
        paren = bracket = brace = 0
        scan = body_start
        next_arm: int | None = None
        while scan < len(body):
            char = body[scan]
            if char == "(":
                paren += 1
            elif char == ")":
                paren -= 1
            elif char == "[":
                bracket += 1
            elif char == "]":
                bracket -= 1
            elif char == "{":
                brace += 1
            elif char == "}":
                brace -= 1
            elif (
                char == "|"
                and paren == bracket == brace == 0
                and body[body.rfind("\n", 0, scan) + 1 : scan].strip() == ""
            ):
                next_arm = scan
                break
            scan += 1
        arm_end = next_arm if next_arm is not None else len(body)
        statements = body[body_start:arm_end]
        newline_prefix = statements[: len(statements) - len(statements.lstrip(" \t"))]
        statements = statements.strip()
        keyword = "default" if pattern == "_" else f"case {pattern}"
        indent = arm.group("indent")
        if statements.startswith("{") and statements.endswith("}"):
            converted = f"{indent}{keyword} {statements}"
        elif "\n" in statements:
            converted = f"{indent}{keyword} {{\n{statements}\n{indent}}}"
        else:
            converted = f"{indent}{keyword} {{ {statements} }}"
        out.append(converted)
        if next_arm is not None:
            out.append("\n")
            cursor = next_arm
        else:
            cursor = len(body)
    return "".join(out)


def convert_match_statements(source: str) -> str:
    out: list[str] = []
    cursor = 0
    match_re = re.compile(r"\bmatch\s*(?P<scrutinee>\([^\n{}]*\)|[^\n{}]+?)\s*\{")
    while True:
        match = match_re.search(source, cursor)
        if match is None:
            out.append(source[cursor:])
            break
        # Skip matches nested in an assembly block; Yul switch/match syntax is
        # outside this migration.
        prefix = source[cursor : match.start()]
        if prefix.rfind("assembly") > prefix.rfind("}"):
            out.append(source[cursor : match.end()])
            cursor = match.end()
            continue
        open_brace = match.end() - 1
        depth = 1
        scan = open_brace + 1
        while scan < len(source) and depth:
            if source[scan] == "{":
                depth += 1
            elif source[scan] == "}":
                depth -= 1
            scan += 1
        if depth:
            out.append(source[cursor:])
            break
        close_brace = scan - 1
        scrutinee = match.group("scrutinee").strip()
        if not (scrutinee.startswith("(") and scrutinee.endswith(")")):
            scrutinee = f"({scrutinee})"
        out.append(source[cursor : match.start()])
        out.append(f"match {scrutinee} {{")
        out.append(convert_match_body(source[open_brace + 1 : close_brace]))
        out.append("}")
        cursor = close_brace + 1
    return "".join(out)


def convert_typed_lets_and_fields(source: str) -> str:
    lines = source.splitlines(keepends=True)
    output: list[str] = []
    assembly_depth = 0
    for original in lines:
        newline = "\n" if original.endswith("\n") else ""
        line = original[:-1] if newline else original
        if assembly_depth == 0:
            # Typed local bindings are unambiguous because they begin with let.
            let_match = re.match(
                r"^(?P<prefix>\s*let\s+[A-Za-z_][A-Za-z0-9_]*\s*:\s*)"
                r"(?P<ty>.*?)(?P<tail>\s*(?:=|:=|;).*)$",
                line,
            )
            if let_match:
                line = let_match.group("prefix") + convert_type(let_match.group("ty")) + let_match.group("tail")
            # Contract fields are also line-oriented (`name: Type [= expr];`).
            field = re.match(
                r"^(?P<prefix>\s*[A-Za-z_][A-Za-z0-9_]*\s*:\s*)"
                r"(?P<ty>.*?)(?P<tail>\s*(?:=\s*.*)?;\s*)$",
                line,
            )
            if field and not line.lstrip().startswith(("case ", "default ")):
                line = field.group("prefix") + convert_type(field.group("ty")) + field.group("tail")
        if assembly_depth == 0 and re.search(r"\bassembly(?:\s*\([^)]*\))?\s*\{", line):
            assembly_depth = max(0, brace_delta(line))
        elif assembly_depth:
            assembly_depth = max(0, assembly_depth + brace_delta(line))
        output.append(line + newline)
    return "".join(output)


def convert_constructor_modifiers(source: str) -> str:
    pattern = re.compile(
        r"(?m)^(?P<indent>[ \t]*)(?P<mods>(?:(?:public|payable)\s+)+)"
        r"(?P<kind>constructor|fallback)(?P<params>\([^\n]*\))"
    )

    def replacement(match: re.Match[str]) -> str:
        mods = " ".join(dict.fromkeys(match.group("mods").split()))
        # public is implicit and diagnosed by the new grammar; preserving it
        # after the parameters lets negative modifier tests keep their intent.
        return (
            f"{match.group('indent')}{match.group('kind')}{match.group('params')}"
            f" {mods}"
        )

    source = pattern.sub(replacement, source)

    fallback = re.compile(
        r"(?m)^(?P<indent>[ \t]*)(?P<prefix>(?:(?:public|payable)\s+)*)"
        r"fallback(?P<params>\([^()\n{}]*\))(?P<tail>[^\n{}]*)\{"
    )

    def canonical_fallback(match: re.Match[str]) -> str:
        modifiers = [
            word
            for word in (match.group("prefix") + " " + match.group("tail")).split()
            if word in {"public", "payable"}
        ]
        modifier_text = "" if not modifiers else " " + " ".join(dict.fromkeys(modifiers))
        return f"{match.group('indent')}fallback{match.group('params')}{modifier_text} {{"

    return fallback.sub(canonical_fallback, source)


def _conditional_keyword(
    ts, start: int, choices: set[str]
) -> int | None:
    """Find a conditional keyword outside nested delimiters."""
    depth = {"(": 0, "[": 0, "{": 0}
    closing = {")": "(", "]": "[", "}": "{"}
    for index in range(start, len(ts)):
        text = ts[index].text
        if text in choices and not any(depth.values()):
            return index
        if text in depth:
            depth[text] += 1
        elif text in closing:
            opener = closing[text]
            if depth[opener]:
                depth[opener] -= 1
            elif text in choices:
                return index
            else:
                return None
    return None


def convert_if_expressions(source: str) -> str:
    """Rewrite legacy `if c then x else y` expressions to ternaries."""
    if SOURCE_MIGRATOR is None:
        return source
    while True:
        ts = SOURCE_MIGRATOR.tokens(source)
        assembly = SOURCE_MIGRATOR.assembly_ranges(ts)
        candidate: tuple[int, int, int, int] | None = None
        # The rightmost legacy if is innermost with respect to another legacy
        # conditional.  Retokenizing after each edit makes nested expressions
        # straightforward and keeps source offsets exact.
        for index, tok in enumerate(ts):
            if tok.text != "if" or SOURCE_MIGRATOR.in_ranges(index, assembly):
                continue
            then = _conditional_keyword(ts, index + 1, {"then"})
            if then is None:
                continue
            otherwise = _conditional_keyword(ts, then + 1, {"else"})
            if otherwise is None:
                continue
            end = _conditional_keyword(
                ts,
                otherwise + 1,
                {";", ",", ")", "]", "}", "then", "else"},
            )
            if end is None:
                end = len(ts)
            no_end = ts[end].start if end < len(ts) else len(source)
            if not source[tok.end : ts[then].start].strip():
                continue
            if not source[ts[then].end : ts[otherwise].start].strip():
                continue
            if not source[ts[otherwise].end : no_end].strip():
                continue
            candidate = (index, then, otherwise, end)
        if candidate is None:
            return source
        index, then, otherwise, end = candidate
        condition = source[ts[index].end : ts[then].start].strip()
        yes = source[ts[then].end : ts[otherwise].start].strip()
        no_end = ts[end].start if end < len(ts) else len(source)
        no = source[ts[otherwise].end : no_end].strip()
        replacement = f"({condition} ? {yes} : {no})"
        source = SOURCE_MIGRATOR.apply_edits(
            source, [(ts[index].start, no_end, replacement)]
        )


def convert_if_statement_conditions(source: str) -> str:
    """Parenthesize non-Yul statement-if conditions."""
    if SOURCE_MIGRATOR is None:
        return source
    ts = SOURCE_MIGRATOR.tokens(source)
    assembly = SOURCE_MIGRATOR.assembly_ranges(ts)
    edits: list[tuple[int, int, str]] = []
    for index, tok in enumerate(ts):
        if tok.text != "if" or SOURCE_MIGRATOR.in_ranges(index, assembly):
            continue
        if index + 1 >= len(ts) or ts[index + 1].text == "(":
            continue
        opening = _conditional_keyword(ts, index + 1, {"{", "then", ";"})
        if opening is None or ts[opening].text != "{" or opening == index + 1:
            continue
        edits.append((ts[index + 1].start, ts[index + 1].start, "("))
        edits.append((ts[opening - 1].end, ts[opening - 1].end, ")"))
    return SOURCE_MIGRATOR.apply_edits(source, edits)


def convert_core_walrus(source: str) -> str:
    """Use `=` for Core bindings/assignments while preserving Yul `:=`."""
    if SOURCE_MIGRATOR is None:
        return source
    ts = SOURCE_MIGRATOR.tokens(source)
    assembly = SOURCE_MIGRATOR.assembly_ranges(ts)
    edits = [
        (tok.start, tok.end, "=")
        for index, tok in enumerate(ts)
        if tok.text == ":=" and not SOURCE_MIGRATOR.in_ranges(index, assembly)
    ]
    return SOURCE_MIGRATOR.apply_edits(source, edits)


def migrate_source(source: str, *, skip_shared_migrator: bool = False) -> str:
    if not SOURCE_MARKER.search(source) and not MATCH_SOURCE_MARKER.search(source):
        return source
    source = convert_imports(source)
    source = convert_data(source)
    source = convert_headers(source)
    source = convert_signature_types(source)
    source = convert_typed_lets_and_fields(source)
    source = convert_constructor_modifiers(source)
    source = convert_match_statements(source)
    source = convert_if_expressions(source)
    source = convert_if_statement_conditions(source)
    source = convert_core_walrus(source)
    # Format templates and nested-comment negative fixtures stay on the local
    # structural passes above.  Their protected placeholders or deliberately
    # malformed tokens are outside the full-source migrator's grammar.
    has_nested_block_comment = re.search(r"/\*(?:(?!\*/).)*/\*", source, re.S) is not None
    if (
        SOURCE_MIGRATOR is not None
        and not skip_shared_migrator
        and "{{" not in source
        and "}}" not in source
        and not has_nested_block_comment
    ):
        warnings: list[str] = []
        try:
            source = SOURCE_MIGRATOR.migrate(source, warnings)
        except (ValueError, IndexError):
            # The local passes still cover the safe subset.  Intentional
            # negative fixtures and format-string placeholders can be outside
            # the full-source migrator's representable grammar.
            pass
    return source


def decode_rust_string(content: str) -> str | None:
    out: list[str] = []
    index = 0
    escapes = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
    while index < len(content):
        if content[index] != "\\":
            out.append(content[index])
            index += 1
            continue
        index += 1
        if index >= len(content):
            return None
        char = content[index]
        if char in escapes:
            out.append(escapes[char])
            index += 1
        elif char == "x" and index + 2 < len(content):
            try:
                out.append(chr(int(content[index + 1 : index + 3], 16)))
            except ValueError:
                return None
            index += 3
        elif char == "u" and index + 1 < len(content) and content[index + 1] == "{":
            close = content.find("}", index + 2)
            if close < 0:
                return None
            try:
                out.append(chr(int(content[index + 2 : close].replace("_", ""), 16)))
            except ValueError:
                return None
            index = close + 1
        elif char == "\n":
            index += 1
            while index < len(content) and content[index] in " \t\r\n":
                index += 1
        else:
            # Unknown escapes may be intentionally invalid Rust in compile-fail
            # support code; leave that literal untouched.
            return None
    return "".join(out)


def encode_rust_string(content: str) -> str:
    out: list[str] = []
    for char in content:
        if char == "\\":
            out.append("\\\\")
        elif char == '"':
            out.append('\\"')
        elif char == "\n":
            out.append("\\n")
        elif char == "\r":
            out.append("\\r")
        elif char == "\t":
            out.append("\\t")
        elif ord(char) < 0x20 or ord(char) == 0x7F:
            out.append(f"\\x{ord(char):02x}")
        else:
            out.append(char)
    return "".join(out)


FORMAT_MACRO_PREFIX = re.compile(r"\b(?:format|format_args)!\s*\(\s*$")
FORMAT_PLACEHOLDER = re.compile(
    r"(?:[0-9]+|[A-Za-z_][A-Za-z0-9_]*)?(?:[!:].*)?\Z", re.S
)


def is_format_macro_literal(text: str, literal_start: int) -> bool:
    """Return whether a literal is the format template of a format macro."""
    return FORMAT_MACRO_PREFIX.search(text, 0, literal_start) is not None


def decode_format_template(template: str) -> tuple[str, list[tuple[str, str]]] | None:
    """Decode literal braces while protecting Rust format placeholders."""
    out: list[str] = []
    placeholders: list[tuple[str, str]] = []
    index = 0
    while index < len(template):
        if template.startswith("{{", index):
            out.append("{")
            index += 2
            continue
        if template.startswith("}}", index):
            out.append("}")
            index += 2
            continue
        if template[index] == "{":
            close = template.find("}", index + 1)
            if close < 0:
                out.append("{")
                index += 1
                continue
            placeholder = template[index : close + 1]
            if FORMAT_PLACEHOLDER.fullmatch(template[index + 1 : close]) is None:
                out.append("{")
                index += 1
                continue
            marker_index = len(placeholders)
            marker = f"__solcore_format_arg_{marker_index}__"
            used_markers = {existing for existing, _ in placeholders}
            while marker in template or marker in used_markers:
                marker_index += 1
                marker = f"__solcore_format_arg_{marker_index}__"
            placeholders.append((marker, placeholder))
            out.append(marker)
            index = close + 1
            continue
        if template[index] == "}":
            out.append("}")
            index += 1
            continue
        out.append(template[index])
        index += 1
    return "".join(out), placeholders


def encode_format_template(
    source: str, placeholders: list[tuple[str, str]]
) -> str | None:
    """Escape source braces and restore protected Rust format placeholders."""
    if any(source.count(marker) != 1 for marker, _ in placeholders):
        return None
    template = source.replace("{", "{{").replace("}", "}}")
    for marker, placeholder in placeholders:
        template = template.replace(marker, placeholder)
    return template


def migrate_rust_strings(text: str) -> tuple[str, int]:
    output: list[str] = []
    cursor = 0
    changed = 0
    opener = re.compile(
        r"(?<![A-Za-z0-9_])(?:(?P<raw_prefix>br|r)(?P<hashes>#{0,16})|(?P<byte>b)?)\""
    )
    while True:
        match = opener.search(text, cursor)
        if not match:
            output.append(text[cursor:])
            break
        content_start = match.end()
        hashes = match.group("hashes")
        if match.group("raw_prefix"):
            close = '"' + (hashes or "")
            content_end = text.find(close, content_start)
            if content_end < 0:
                output.append(text[cursor:])
                break
            encoded_content = text[content_start:content_end]
            decoded = encoded_content
        else:
            scan = content_start
            while scan < len(text):
                if text[scan] == "\\":
                    scan += 2
                    continue
                if text[scan] == '"':
                    break
                scan += 1
            if scan >= len(text):
                output.append(text[cursor:])
                break
            close = '"'
            content_end = scan
            encoded_content = text[content_start:content_end]
            decoded = decode_rust_string(encoded_content)
            if decoded is None:
                output.append(text[cursor : content_end + 1])
                cursor = content_end + 1
                continue
        output.append(text[cursor:content_start])
        prefix = text[: match.start()]
        preserved_region = prefix.rfind(PRESERVE_LITERALS_BEGIN_MARKER) > prefix.rfind(
            PRESERVE_LITERALS_END_MARKER
        )
        preserve_next = PRESERVE_NEXT_LITERAL_MARKER in text[cursor : match.start()]
        format_template = None
        if not (preserved_region or preserve_next) and is_format_macro_literal(
            text, match.start()
        ):
            format_template = decode_format_template(decoded)
        if preserved_region or preserve_next:
            migrated = decoded
        elif format_template is None:
            migrated = migrate_source(decoded)
        else:
            source, placeholders = format_template
            migrated_source = migrate_source(source, skip_shared_migrator=True)
            migrated = encode_format_template(migrated_source, placeholders)
            if migrated is None:
                migrated = decoded
        changed_literal = migrated != decoded
        if changed_literal:
            changed += 1
        if not changed_literal:
            output.append(encoded_content)
        elif match.group("raw_prefix"):
            output.append(migrated)
        else:
            output.append(encode_rust_string(migrated))
        output.append(close)
        cursor = content_end + len(close)
    return "".join(output), changed


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    parser.add_argument("paths", nargs="+", type=Path)
    args = parser.parse_args()
    total = 0
    for path in args.paths:
        original = path.read_text()
        migrated, changed = migrate_rust_strings(original)
        if changed:
            total += changed
            print(f"{path}: {changed} raw string(s)")
            if args.write:
                path.write_text(migrated)
    print(f"changed raw strings: {total}")


if __name__ == "__main__":
    main()
