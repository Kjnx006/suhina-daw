# Copyright (C) 2026 The Suhina contributors
# SPDX-License-Identifier: GPL-3.0-or-later

"""Tests for scripts/inline_tests.py: finding long inline test modules and moving them out."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import inline_tests  # noqa: E402


def source(test_lines: int, name: str = "tests") -> str:
    body = "\n".join(f"    // line {n}" for n in range(test_lines - 3))
    return f'fn code() {{}}\n\n#[cfg(test)]\nmod {name} {{\n{body}\n}}\n\nfn after() {{\n    let s = "x\n  y";\n}}\n'


class InlineTests(unittest.TestCase):
    def test_a_module_over_the_limit_is_long_and_one_at_it_is_not(self):
        self.assertIsNone(inline_tests.long_module(source(inline_tests.LIMIT)))
        self.assertIsNotNone(inline_tests.long_module(source(inline_tests.LIMIT + 1)))

    def test_a_module_without_cfg_test_right_above_it_does_not_count(self):
        text = source(400).replace("#[cfg(test)]\n", "#[cfg(test)]\n#[allow(dead_code)]\n")
        self.assertEqual(inline_tests.test_modules(text), [])

    def test_moving_leaves_a_path_module_and_the_body_under_the_header(self):
        text = source(200, "checks")
        result = inline_tests.moved(text, "foo_tests.rs")
        assert result is not None
        new_source, tests = result
        self.assertIn('#[cfg(test)]\n#[path = "foo_tests.rs"]\nmod checks;\n', new_source)
        self.assertTrue(new_source.startswith("fn code() {}\n"))
        self.assertIn('fn after() {\n    let s = "x\n  y";\n}', new_source)
        self.assertNotIn("// line", new_source)
        self.assertTrue(tests.startswith(inline_tests.HEADER + "\n    // line 0\n"))
        self.assertTrue(tests.endswith(f"    // line {200 - 4}\n"))

    def test_a_file_without_a_long_module_is_not_moved(self):
        self.assertIsNone(inline_tests.moved(source(20), "foo_tests.rs"))

    def test_lib_and_main_get_a_tests_file_named_after_them(self):
        self.assertEqual(inline_tests.test_file_name(Path("crates/x/src/lib.rs")), "lib_tests.rs")
        self.assertEqual(inline_tests.test_file_name(Path("crates/x/src/view/menu.rs")), "menu_tests.rs")


if __name__ == "__main__":
    unittest.main()
