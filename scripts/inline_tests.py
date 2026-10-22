#!/usr/bin/env python3
# Copyright (C) 2026 The Suhina contributors
# SPDX-License-Identifier: GPL-3.0-or-later

"""Keeps long test modules out of Rust source files (issue #768, part 5).

    python scripts/inline_tests.py --check      # list inline test modules over the limit
    python scripts/inline_tests.py move FILE... # move each file's long test module out

An agent reading a source file reads its inline `#[cfg(test)] mod tests { ... }` too, so a
test module over `LIMIT` lines lives in a file of its own beside the source:
`foo.rs` keeps

    #[cfg(test)]
    #[path = "foo_tests.rs"]
    mod tests;

and `foo_tests.rs` holds the module's body under the licence header, formatted by `rustfmt`
(which re-indents the code and leaves string literals as they are). The tests themselves do
not change. `move` does exactly that for the first inline test module over
the limit in each file it is given (`lib.rs` and `main.rs` give `lib_tests.rs` and
`main_tests.rs`), and refuses a file whose test file exists already.

`--check` fails on any tracked `src/` file with an inline test module over the limit, except
those in `WAITING`: files that open pull requests are still changing, so that moving their tests
now would make every one of those pull requests conflict. Each is moved in a pull request of its
own once they have merged, and its entry is removed then; an entry for a file that no longer
has a long module fails the check too, so the list cannot go stale.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(
    subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, check=True, text=True).stdout.strip()
)

# Longest inline test module allowed, in lines, from `#[cfg(test)]` to its closing brace.
LIMIT = 150

HEADER = "// Copyright (C) 2026 The Suhina contributors\n// SPDX-License-Identifier: GPL-3.0-or-later\n"

# Files whose tests wait for the open pull requests that change them (see the docstring).
WAITING = {
    "crates/automation/src/confine.rs",
    "crates/automation/src/render.rs",
    "crates/device-sdk/src/lib.rs",
    "crates/engine/src/launcher.rs",
    "crates/engine/src/lib.rs",
    "crates/engine/src/stretch.rs",
    "crates/media/src/lib.rs",
    "crates/model/src/lib.rs",
    "crates/plugins/src/protocol.rs",
    "crates/session/src/rack.rs",
    "crates/ui/src/app.rs",
    "crates/ui/src/app/splash.rs",
    "crates/ui/src/arrangement/arrangement_view.rs",
    "crates/ui/src/arrangement/automation_lanes.rs",
    "crates/ui/src/arrangement/timeline.rs",
    "crates/ui/src/browser/sample_map.rs",
    "crates/ui/src/detail/detail_view.rs",
    "crates/ui/src/detail/device_canvas.rs",
    "crates/ui/src/detail/piano_roll.rs",
    "crates/ui/src/detail/piano_roll_ghosts.rs",
    "crates/ui/src/kit/theme.rs",
    "crates/ui/src/settings.rs",
}

OPENING = re.compile(r"^(\s*)(?:pub(?:\([\w:]+\))? )?mod (\w+) \{\s*$")


def test_modules(text: str) -> list[tuple[int, int, str, str]]:
    """Each inline test module in `text`: (index of its `#[cfg(test)]` line, index of its
    closing brace, its indent, its name). A module counts when `#[cfg(test)]` stands on the line
    right above `mod name {` and its closing brace is the first line holding only `}` at the same
    indent."""
    lines = text.split("\n")
    found = []
    for i, line in enumerate(lines[:-1]):
        opening = OPENING.match(lines[i + 1])
        if line.strip() != "#[cfg(test)]" or not opening:
            continue
        indent, name = opening.group(1), opening.group(2)
        end = next((j for j in range(i + 2, len(lines)) if lines[j] == indent + "}"), None)
        if end is not None:
            found.append((i, end, indent, name))
    return found


def long_module(text: str) -> tuple[int, int, str, str] | None:
    """The first inline test module of `text` over `LIMIT` lines, if any."""
    return next((m for m in test_modules(text) if m[1] - m[0] + 1 > LIMIT), None)


def moved(text: str, test_file: str) -> tuple[str, str] | None:
    """`text` with its long test module replaced by a `#[path]` module, and the test file's
    text, or `None` when it has no long module."""
    module = long_module(text)
    if module is None:
        return None
    start, end, indent, name = module
    lines = text.split("\n")
    # The body keeps its indent: rustfmt takes it off the code, and leaves string literals that
    # run over several lines exactly as they are, as a hand-made dedent would not.
    body = lines[start + 2 : end]
    while body and not body[0].strip():
        body.pop(0)
    while body and not body[-1].strip():
        body.pop()
    stub = [f"{indent}#[cfg(test)]", f'{indent}#[path = "{test_file}"]', f"{indent}mod {name};"]
    source = "\n".join(lines[:start] + stub + lines[end + 1 :])
    return source, HEADER + "\n" + "\n".join(body) + "\n"


def test_file_name(path: Path) -> str:
    return f"{path.stem}_tests.rs"


def tracked_sources() -> list[str]:
    out = subprocess.run(["git", "ls-files", "*.rs"], cwd=ROOT, capture_output=True, check=True, text=True).stdout
    return [p for p in out.split() if "/src/" in f"/{p}" and not p.startswith("third_party/")]


def check(paths: list[str]) -> list[str]:
    """What `--check` reports for `paths` (relative to the repository root)."""
    problems = []
    long = set()
    for path in paths:
        module = long_module((ROOT / path).read_text(encoding="utf-8"))
        if module is None:
            continue
        long.add(path)
        if path not in WAITING:
            lines = module[1] - module[0] + 1
            problems.append(
                f"{path}:{module[0] + 1}: an inline test module of {lines} lines (over {LIMIT}); "
                f"move it out with `python scripts/inline_tests.py move {path}`"
            )
    for path in sorted(WAITING - long):
        problems.append(f"{path} is in WAITING but has no inline test module over {LIMIT} lines; remove its entry")
    return problems


def move(path: Path) -> str:
    """Moves `path`'s long test module out; returns what was done."""
    test_path = path.with_name(test_file_name(path))
    if test_path.exists():
        return f"{path}: {test_path.name} exists already, left as it is"
    result = moved(path.read_text(encoding="utf-8"), test_path.name)
    if result is None:
        return f"{path}: no inline test module over {LIMIT} lines"
    source, tests = result
    path.write_text(source, encoding="utf-8")
    test_path.write_text(tests, encoding="utf-8")
    subprocess.run(["rustfmt", "--edition", "2024", str(test_path)], check=True)
    return f"{path}: tests moved to {test_path.name}"


def main(argv: list[str]) -> int:
    if argv[:1] == ["--check"]:
        problems = check(tracked_sources())
        for problem in problems:
            print(problem)
        return 1 if problems else 0
    if argv[:1] == ["move"] and len(argv) > 1:
        for name in argv[1:]:
            print(move(Path(name)))
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
