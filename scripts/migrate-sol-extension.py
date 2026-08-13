#!/usr/bin/env python3
"""Rename tracked Solcore sources from .solc to .sol and update references."""

from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[1]
TEXT_EXTENSIONS = {
    ".c",
    ".css",
    ".h",
    ".html",
    ".js",
    ".json",
    ".lua",
    ".md",
    ".mjs",
    ".py",
    ".rs",
    ".sh",
    ".snap",
    ".toml",
    ".ts",
    ".tsx",
    ".tsv",
    ".txt",
    ".vim",
    ".el",
    ".yaml",
    ".yml",
}
TEXT_NAMES = {"Makefile"}

# These spellings intentionally exercise rejection of the retired extension.
# Protect them so the one-shot migration remains idempotent when rerun while
# reviewing or extending the cut-over.
REFERENCE_EXCEPTIONS = {
    Path("crates/driver/src/standard_json.rs"): ('"main.solc"',),
}


def tracked_files() -> list[Path]:
    output = subprocess.check_output(
        ["git", "ls-files", "-z"], cwd=ROOT
    ).decode("utf-8")
    return [ROOT / name for name in output.rstrip("\0").split("\0") if name]


def rename_sources(files: list[Path]) -> None:
    for source in files:
        if source.suffix != ".solc" or not source.exists():
            continue
        destination = source.with_suffix(".sol")
        if destination.exists():
            raise RuntimeError(f"refusing to overwrite {destination.relative_to(ROOT)}")
        source.rename(destination)


def update_references(files: list[Path]) -> None:
    for path in files:
        if path.suffix == ".solc":
            path = path.with_suffix(".sol")
        if not path.exists() or not path.is_file():
            continue
        if path.suffix not in TEXT_EXTENSIONS and path.name not in TEXT_NAMES:
            continue
        try:
            original = path.read_text()
        except UnicodeDecodeError:
            continue
        relative = path.relative_to(ROOT)
        protected = original
        placeholders: list[tuple[str, str]] = []
        for index, spelling in enumerate(REFERENCE_EXCEPTIONS.get(relative, ())):
            marker = f"__SOLCORE_EXTENSION_MIGRATION_EXCEPTION_{index}__"
            if marker in protected:
                raise RuntimeError(f"migration marker already present in {relative}")
            protected = protected.replace(spelling, marker)
            placeholders.append((marker, spelling))
        # Match an extension-like suffix, not the `.solc` prefix in names such
        # as `settings.solcore` or TextMate's `source.solcore` scope.
        updated = re.sub(r"\.solc(?=$|[^A-Za-z0-9_-])", ".sol", protected)
        for marker, spelling in placeholders:
            updated = updated.replace(marker, spelling)
        if updated != original:
            path.write_text(updated)


def main() -> None:
    files = tracked_files()
    rename_sources(files)
    update_references(files)


if __name__ == "__main__":
    main()
