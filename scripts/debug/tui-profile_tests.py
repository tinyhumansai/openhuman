#!/usr/bin/env python3
"""Deterministic parser checks; run with python3 scripts/debug/tui-profile_tests.py."""
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("tui_profile", Path(__file__).with_name("tui-profile.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ScreenTests(unittest.TestCase):
    def test_differential_frames_reconstruct_marker_and_erase_placeholder(self):
        screen = MODULE.Screen(80,24)
        screen.feed(b"\x1b[21;2HDescribe a task")
        screen.feed(b"\x1b[21;2HAAA\x1b[0K")
        screen.feed(b"\x1b[21;5HBBB")
        self.assertTrue(screen.contains(b"AAABBB"))
        self.assertFalse(screen.contains(b"Describe a task"))

    def test_split_cursor_sequence_styles_and_osc_do_not_add_visible_text(self):
        screen = MODULE.Screen(80,24)
        screen.feed(b"\x1b]title;hidden")
        screen.feed(b"\x1b\\\x1b[21;")
        screen.feed(b"2H\x1b[38;2;10;20;30mvisible\x1b[0m")
        self.assertTrue(screen.contains(b"visible"))
        self.assertFalse(screen.contains(b"hidden"))
        self.assertEqual(screen.cells[(1,20)],"v")

    def test_split_utf8_wide_and_combining_characters_preserve_cell_positions(self):
        screen = MODULE.Screen(80,24)
        raw = "界e\u0301X".encode("utf-8")
        for byte in raw:
            screen.feed(bytes([byte]))
        self.assertEqual(screen.cells[(0,0)],"界")
        self.assertEqual(screen.cells[(2,0)],"e")
        self.assertEqual(screen.cells[(3,0)],"X")

    def test_cursor_motion_and_screen_clear_remove_old_marker(self):
        screen = MODULE.Screen(80,24)
        screen.feed(b"\x1b[4;3Hfirst\x1b[5G\x1b[1DXY")
        self.assertEqual(screen.cells[(3,3)],"X")
        self.assertEqual(screen.cells[(4,3)],"Y")
        screen.feed(b"\x1b[2J\x1b[1;1Hnext")
        self.assertFalse(screen.contains(b"first"))
        self.assertTrue(screen.contains(b"next"))


if __name__ == "__main__":
    unittest.main()
