"""Unix terminal integration checks; run after cargo build, using only Python's stdlib."""

import base64
import codecs
import errno
import fcntl
import json
import mmap
import os
from pathlib import Path
import pty
import re
import select
import shlex
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import threading
import unicodedata
import unittest


BINARY = Path(__file__).resolve().parents[1] / "target/debug/dift"


def has_tree_badge(text, name, badges):
    """Match a tree row independently of the space reserved for its basename."""
    return re.search(rf"{re.escape(name)} +{re.escape(badges)} [ │┃█║]", text) is not None


def has_tree_badge_color(screen, name, badge, color):
    """Check the status cell on a tree row, including its foreground."""
    for y, row in enumerate(screen.rows):
        match = re.search(rf"{re.escape(name)} +({re.escape(badge)}) [ │┃█║]", "".join(row))
        if match and screen.foregrounds[y][match.start(1)] == color:
            return True
    return False


def has_selection(text, name):
    """Check the focused tree row independently of transient notifications."""
    rows = text.splitlines()
    tree_width = min(len(rows[0]) // 3, 34)
    return any(
        row.startswith("▎") and re.search(rf"(?:^| ){re.escape(name)} +", row[1:tree_width])
        for row in rows[:-3]
    )


def has_split_edit(text, before, after):
    """Find both sides of an edited line on the same terminal row."""
    return any(before in row and after in row and "│" in row for row in text.splitlines()[:-3])


class Screen:
    """Small VT screen reader for the cursor/erase sequences emitted by Ratatui."""

    def __init__(self, width=120, height=35):
        self.width, self.height = width, height
        self.rows = [[" "] * width for _ in range(height)]
        self.foregrounds = [[None] * width for _ in range(height)]
        self.inversions = [[False] * width for _ in range(height)]
        self.foreground = None
        self.inverted = False
        self.x = self.y = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            if self.pending.startswith("\x1b]"):
                terminator = re.search(r"\x07|\x1b\\", self.pending)
                if terminator is None:
                    return
                self.pending = self.pending[terminator.end():]
                continue
            if self.pending.startswith("\x1b["):
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending)
                if match is None:
                    return
                raw, _, final = match.groups()
                self.pending = self.pending[match.end():]
                if raw.startswith(("?", ">", "<", "=")):
                    continue
                args = [int(p or "0") for p in raw.split(";")]
                first = args[0] or 1
                if final in "Hf":
                    self.y = first - 1
                    self.x = (args[1] or 1) - 1 if len(args) > 1 else 0
                elif final == "G":
                    self.x = first - 1
                elif final == "d":
                    self.y = first - 1
                elif final == "A":
                    self.y = max(0, self.y - first)
                elif final == "B":
                    self.y += first
                elif final == "C":
                    self.x += first
                elif final == "D":
                    self.x = max(0, self.x - first)
                elif final == "J" and args[0] in (2, 3):
                    self.rows = [[" "] * self.width for _ in range(self.height)]
                    self.foregrounds = [[None] * self.width for _ in range(self.height)]
                    self.inversions = [[False] * self.width for _ in range(self.height)]
                elif final == "m":
                    self.set_colors(args)
                elif final == "K" and self.y < self.height:
                    start = 0 if args[0] in (1, 2) else self.x
                    end = self.width if args[0] in (0, 2) else self.x + 1
                    self.rows[self.y][start:end] = [" "] * (end - start)
                    self.foregrounds[self.y][start:end] = [self.foreground] * (end - start)
                    self.inversions[self.y][start:end] = [False] * (end - start)
            elif self.pending.startswith("\x1b"):
                if len(self.pending) < 2:
                    return
                length = 3 if self.pending[1] in "()" else 2
                if len(self.pending) < length:
                    return
                self.pending = self.pending[length:]
            else:
                ch, self.pending = self.pending[0], self.pending[1:]
                if ch == "\r":
                    self.x = 0
                elif ch == "\n":
                    self.y += 1
                elif ch == "\b":
                    self.x = max(0, self.x - 1)
                # Private-use characters (Co), including Nerd Font glyphs,
                # occupy terminal cells just like other printable characters.
                elif unicodedata.category(ch) not in ("Cc", "Cf", "Cs"):
                    width = 0 if unicodedata.combining(ch) else (2 if unicodedata.east_asian_width(ch) in "WF" else 1)
                    if 0 <= self.y < self.height and 0 <= self.x < self.width:
                        self.rows[self.y][self.x] = ch
                        self.foregrounds[self.y][self.x] = self.foreground
                        self.inversions[self.y][self.x] = self.inverted
                    self.x += width

    def set_colors(self, args):
        position = 0
        while position < len(args):
            code = args[position]
            if code in (0, 27):
                self.inverted = False
            elif code == 7:
                self.inverted = True
            if code in (0, 39):
                self.foreground = None
            elif 30 <= code <= 37 or 90 <= code <= 97:
                self.foreground = code
            elif code in (38, 48, 58) and position + 1 < len(args):
                mode = args[position + 1]
                size = {2: 3, 5: 1}.get(mode, 0)
                color = tuple(args[position + 2:position + 2 + size])
                if code == 38 and len(color) == size and size:
                    self.foreground = color
                position += 1 + size
            position += 1

    @property
    def text(self):
        return "\n".join("".join(row) for row in self.rows)

    def locate(self, text):
        for y, row in enumerate(self.rows):
            x = "".join(row).find(text)
            if x >= 0:
                return x, y
        raise AssertionError(f"Missing {text!r} in terminal:\n{self.text}")

    def highlighted_code(self):
        return [selected for row, inversions in zip(self.rows, self.inversions)
                if (selected := "".join(cell for cell, inverted in zip(row, inversions) if inverted))]


class Session:
    def __init__(self, cwd, config, env=None, args=()):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.screen = Screen()
        self.resize(120, 35)
        self.pid = os.fork()
        if self.pid == 0:
            os.close(self.master)
            os.login_tty(self.slave)
            os.chdir(cwd)
            # Color assertions must not depend on the invoking shell's NO_COLOR.
            child_env = {**os.environ, "TERM": "xterm-256color", "NO_COLOR": None, **(env or {})}
            os.execve(str(BINARY), [str(BINARY), "--config", str(config), *args],
                      {key: value for key, value in child_env.items() if value is not None})
        self.status = None
        self.raw = bytearray()

    def resize(self, width, height):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        self.screen = Screen(width, height)

    def pump(self, timeout=0.1):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                data = os.read(self.master, 1 << 20)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                return
            self.raw.extend(data)
            self.screen.feed(data)

    def wait(self, text, timeout=8):
        self.wait_for(lambda screen: text in screen, repr(text), timeout)

    def wait_selection(self, name, timeout=8):
        self.wait_for(lambda text: has_selection(text, name),
                      f"selected {name!r}", timeout)

    def wait_for(self, predicate, description, timeout=8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump()
            if predicate(self.screen.text):
                return
        raise AssertionError(f"Missing {description} in terminal:\n{self.screen.text}")

    def send(self, keys):
        os.write(self.master, keys.encode())

    def advance(self, duration):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            self.pump(min(0.05, max(0, deadline - time.monotonic())))

    def stop(self):
        # Close an input prompt or help before using the normal quit binding.
        self.send("\x1b")
        self.advance(0.05)
        self.send("q")
        deadline = time.monotonic() + 3
        while self.status is None and time.monotonic() < deadline:
            self.pump()
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid:
                self.status = os.waitstatus_to_exitcode(status)
        if self.status is None:
            os.kill(self.pid, signal.SIGKILL)
            _, status = os.waitpid(self.pid, 0)
            self.status = os.waitstatus_to_exitcode(status)
        return self.status

    def close(self):
        if self.status is None:
            self.stop()
        os.close(self.master)
        os.close(self.slave)


class TerminalSmoke(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="dift-terminal-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git("init", "--initial-branch=main")
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text('fn main() { println!("original"); }\n')
        self.git("add", ".")
        self.git("commit", "-m", "initial")
        (self.root / "src/main.rs").write_text('fn main() { println!("staged"); }\n')
        self.git("add", ".")
        (self.root / "src/main.rs").write_text('fn main() { println!("working"); }\n')
        (self.root / "notes.txt").write_text("untracked note\n")
        self.start_session()

    def start_session(self, cwd=None, reconcile=3600, env=None, args=()):
        # Event tests must complete long before the periodic fallback can run.
        config = self.root / ".git/terminal-config.kdl"
        config.write_text(f"watch {{ reconcile-seconds {reconcile}; }}\n")
        self.session = Session(cwd or self.root / "src", config, env=env, args=args)
        self.addCleanup(self.session.close)
        self.session.wait("Watching" if not args else "Snapshot")

    def restart_session(self, cwd=None, reconcile=3600):
        self.session.stop()
        self.start_session(cwd, reconcile)

    def git(self, *args, cwd=None):
        return subprocess.run([
            "git", "-C", str(cwd or self.root), "-c", "user.name=Terminal Test",
            "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false",
            "-c", "core.hooksPath=/nonexistent-dift-hooks", *args
        ], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def test_drag_highlights_only_code_and_keeps_it_after_release(self):
        # Given
        (self.root / "selection_demo.rs").write_text("first_selection_line\nsecond_selection_line\n")
        self.session.wait("selection_demo.rs")
        self.session.send("fselection_demo\r")
        self.session.wait("second_selection_line")
        self.session.advance(0.4)
        x1, y1 = self.session.screen.locate("first_selection_line")
        x2, y2 = self.session.screen.locate("second_selection_line")
        x2 += len("second_selection_line") - 1
        # When
        self.session.send(f"\x1b[<0;{x1 + 1};{y1 + 1}M")
        self.session.send(f"\x1b[<32;{x2 + 1};{y2 + 1}M")
        self.session.send(f"\x1b[<0;{x2 + 1};{y2 + 1}m")
        self.session.wait_for(lambda _: self.session.screen.highlighted_code() == [
            "first_selection_line", "second_selection_line"], "code-only drag highlight")
        self.session.advance(0.1)
        # Then
        self.assertEqual(self.session.screen.highlighted_code(), ["first_selection_line", "second_selection_line"])
        payloads = re.findall(rb"\x1b\]52;c;([A-Za-z0-9+/=]*)\x1b\\", self.session.raw)
        self.assertEqual(len(payloads), 1)
        self.assertEqual(base64.b64decode(payloads[0], validate=True),
                         b"first_selection_line\nsecond_selection_line")

    def spacing_change(self):
        (self.root / "spacing.txt").write_text("one two\n")
        self.git("add", ".")
        self.git("commit", "-m", "spacing fixture")
        (self.root / "spacing.txt").write_text("one  two\n")
        self.session.wait("spacing.txt")
        self.session.wait_for(lambda text: has_tree_badge(text, "All changes", "1"), "one changed file in the tree")

    def test_whitespace_toggle_hides_files_and_reports_counts(self):
        # Given
        self.spacing_change()
        config = self.root / ".git/terminal-config.kdl"
        original = config.read_bytes()
        # When
        self.session.send("w")
        self.session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")
        # Then
        self.assertNotIn("spacing.txt", self.session.screen.text)
        self.assertNotIn("files", self.session.screen.text.splitlines()[-2])
        self.assertNotIn("whitespace", self.session.screen.text.splitlines()[-2])
        self.assertEqual(config.read_bytes(), original)

    def test_whitespace_toggle_restores_exact_changes(self):
        # Given
        self.spacing_change()
        self.session.send("w")
        self.session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")
        # When
        self.session.send("w")
        self.session.wait("Whitespace: exact · 0 hidden by whitespace")
        self.session.wait("spacing.txt")
        # Then
        self.session.wait("one  two")

    def test_rapid_whitespace_toggles_apply_the_final_mode(self):
        # Given
        self.spacing_change()
        # When
        self.session.send("www")
        self.session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")
        # Then
        self.assertNotIn("spacing.txt", self.session.screen.text)

    def test_watch_reveals_new_content_in_a_whitespace_hidden_file(self):
        # Given
        self.spacing_change()
        self.session.send("w")
        self.session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")
        self.session.wait_for(lambda text: "Whitespace:" not in text, "whitespace toast expired", timeout=4)
        # When
        (self.root / "spacing.txt").write_text("SUBSTANTIVE_EDIT\n")
        self.session.wait("SUBSTANTIVE_EDIT")
        # Then
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "1"))
        self.assertNotIn("Whitespace:", self.session.screen.text)

    def test_configured_ignore_all_is_used_at_startup(self):
        # Given
        self.spacing_change()
        self.session.stop()
        (self.root / "spacing.txt").write_text("onetwo\n")
        config = self.root / ".git/terminal-config.kdl"
        config.write_text('diff { whitespace "ignore-all"; }\n')
        # When
        self.session = Session(self.root, config)
        self.addCleanup(self.session.close)
        self.session.wait("Watching")
        # Then
        self.assertNotIn("spacing.txt", self.session.screen.text)
        self.assertIn("No changes", self.session.screen.text)
        self.assertNotIn("Whitespace:", self.session.screen.text)

    def test_search_typing_w_keeps_exact_comparison(self):
        # Given
        self.spacing_change()
        # When
        self.session.send("/w")
        self.session.wait("Search › w")
        # Then
        self.assertIn("spacing.txt", self.session.screen.text)
        self.assertIn("one  two", self.session.screen.text)
        self.assertNotIn("Whitespace:", self.session.screen.text)

    def test_filtered_file_counts_stay_in_the_tree_and_out_of_the_status(self):
        # Given
        self.session.wait_for(lambda text: has_tree_badge(text, "All changes", "2"), "two changed files in the tree")
        # When
        self.session.send("fnotes\r")
        self.session.wait("filter: notes")
        # Then
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "1/2"))
        self.assertNotIn("files", self.session.screen.text.splitlines()[-2])

    def external_ignore(self, reconcile):
        directory = tempfile.TemporaryDirectory(prefix="dift-external-ignore-")
        self.addCleanup(directory.cleanup)
        ignore = Path(directory.name) / "ignore"
        ignore.write_text("external.txt\n")
        self.git("config", "core.excludesFile", str(ignore))
        (self.root / "external.txt").write_text("EXTERNAL_CHANGE\n")
        self.restart_session(reconcile=reconcile)
        self.assertNotIn("external.txt", self.session.screen.text)
        return ignore

    def add_submodule(self):
        directory = tempfile.TemporaryDirectory(prefix="dift-submodule-source-")
        self.addCleanup(directory.cleanup)
        source = Path(directory.name)
        self.git("init", "--initial-branch=main", cwd=source)
        self.git("commit", "--allow-empty", "-m", "initial", cwd=source)
        self.git("-c", "protocol.file.allow=always", "submodule", "add", str(source), "module")
        self.git("add", ".")
        self.git("commit", "-m", "add module")
        self.restart_session()
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "0"))

    @staticmethod
    def write_burst(path):
        for index in range(40):
            path.write_text(f"burst {index}\n")
            time.sleep(0.02)
        path.write_text("BURST_FINAL\n")

    def syntax_fixture(self):
        source = "".join(f"export const item_{i}: Record<string, number> = {{ count: {i} }};\n" for i in range(600))
        path = self.root / "large.ts"
        path.write_text(source + 'export const last = "INITIAL";\n')
        self.git("add", "large.ts")
        self.git("commit", "-m", "syntax fixture")
        self.restart_session()
        return path, source

    def test_root_shows_all_categories_in_order_from_subdirectory(self):
        text = self.session.screen.text
        self.assertLess(text.index("UNSTAGED"), text.index("UNTRACKED"))
        self.assertLess(text.index("UNTRACKED"), text.index("STAGED", text.index("UNTRACKED")))
        self.assertIn("notes.txt", text)

    def test_file_navigation_skips_directories_and_layout_keeps_headers(self):
        self.session.send("ns")
        self.session.wait_selection("main.rs")
        self.session.wait_for(lambda text: has_split_edit(text, "staged", "working"), "side-by-side diff content")
        self.assertIn("UNSTAGED", self.session.screen.text)
        self.assertNotIn("Layout:", self.session.screen.text)
        self.session.send("n")
        self.session.wait_selection("notes.txt")
        self.session.send("p")
        self.session.wait_selection("main.rs")

    def test_whitespace_toast_expires_while_idle_and_restores_the_content(self):
        # Given
        self.spacing_change()
        self.session.send("w")
        self.session.wait("Whitespace: ignore-changes · 1 hidden by whitespace")
        self.session.send("w")
        self.session.wait("Whitespace: exact · 0 hidden by whitespace")
        self.session.wait("one  two")
        self.session.wait("Watching")
        # When
        self.session.wait_for(lambda text: "Whitespace:" not in text, "whitespace toast expired", timeout=4)
        self.session.advance(0.1)
        size = len(self.session.raw)
        self.session.advance(0.2)
        # Then
        self.assertIn("one  two", self.session.screen.text)
        self.assertNotIn("whitespace", self.session.screen.text.splitlines()[-2])
        self.assertEqual(len(self.session.raw), size)

    def test_filter_changes_both_tree_and_aggregate_diff(self):
        self.session.send("fnotes\r")
        self.session.wait_for(lambda text: has_tree_badge(text, "All changes", "1/2"),
                              "filtered count beside All changes")
        self.session.wait("Watching")
        self.assertIn("untracked note", self.session.screen.text)
        self.assertNotIn("main.rs", self.session.screen.text)

    def test_tree_guide_follows_the_selected_files_parent(self):
        # Given
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("n")
        self.session.wait_selection("notes.txt")
        # Then
        self.assertEqual(self.session.screen.rows[1][1], "│")
        self.assertEqual(self.session.screen.rows[2][1], "│")
        self.assertEqual(self.session.screen.rows[2][3], " ")
        self.assertEqual(self.session.screen.rows[3][1], "│")

    def test_filter_reveals_a_collapsed_folder_and_clearing_restores_it(self):
        # Given
        self.session.send("jhfmain\r")
        self.session.wait_for(lambda text: has_tree_badge(text, "All changes", "1/2"),
                              "filtered count beside All changes")
        self.session.wait("filter: main")
        self.session.send("l")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("\x1b")
        # Then
        self.session.wait_for(lambda text: has_selection(text, "src") and "filter: main" not in text,
                              "restored folder selection")
        self.assertRegex(self.session.screen.text, r"▸ src +1")
        self.assertFalse(has_tree_badge(self.session.screen.text, "main.rs", "MM"))

    def test_nerd_font_config_emits_icons_and_aligned_badges(self):
        # Given
        self.session.stop()
        config = self.root / ".git/terminal-config.kdl"
        source = 'appearance { icons "nerd"; }\n'
        config.write_text(source)
        # When
        self.session = Session(self.root, config)
        self.addCleanup(self.session.close)
        self.session.wait("Watching")
        # Then
        self.assertIn(" All changes", self.session.screen.text)
        self.assertIn(" src", self.session.screen.text)
        self.assertNotIn("▾", self.session.screen.text)
        self.assertNotIn("▸", self.session.screen.text)
        self.assertIn(" main.rs", self.session.screen.text)
        self.assertTrue(has_tree_badge(self.session.screen.text, "main.rs", "MM"))
        self.assertEqual(config.read_text(), source)

    def test_search_is_literal_and_input_keeps_quit_character(self):
        self.session.resize(160, 35)
        self.session.send("/working\r")
        self.session.wait("/working: 1 matches")
        self.session.send("/q")
        self.session.wait("Search › workingq")
        self.session.send("\x1b")
        self.session.wait("/working: 1 matches")

    def test_watch_refreshes_content_without_a_status_change(self):
        (self.root / "src/main.rs").write_text('fn main() { println!("refreshed"); }\n')
        self.session.wait("refreshed")
        self.assertIn("UNSTAGED", self.session.screen.text)

    def test_mapped_write_refreshes_tree_and_diff(self):
        # Given
        path = self.root / "src/main.rs"
        # When
        with path.open("r+b") as stream, mmap.mmap(stream.fileno(), 0) as mapped:
            mapped[:] = mapped[:].replace(b"working", b"MAPPED!")
            mapped.flush()
        # Then
        self.session.wait("MAPPED!", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "main.rs", "MM"), self.session.screen.text)

    def test_submodule_head_change_refreshes_a_clean_tree(self):
        # Given
        self.add_submodule()
        # When
        self.git("commit", "--allow-empty", "-m", "new module commit", cwd=self.root / "module")
        # Then
        self.session.wait("Submodule change", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "1"))
        self.assertTrue(has_tree_badge(self.session.screen.text, "module", "M"), self.session.screen.text)

    def test_atomic_save_refreshes_the_selected_file(self):
        # Given
        self.session.send("n")
        self.session.wait_selection("main.rs")
        temporary = self.root / "src/save.tmp"
        temporary.write_text("ATOMIC_SAVE\n")
        # When
        os.replace(temporary, self.root / "src/main.rs")
        # Then
        self.session.wait("ATOMIC_SAVE", timeout=4)
        self.assertTrue(has_selection(self.session.screen.text, "main.rs"))

    def test_untracked_creation_updates_both_panes(self):
        # Given
        path = self.root / "new.txt"
        # When
        path.write_text("NEW_FILE_CONTENT\n")
        # Then
        self.session.wait("NEW_FILE_CONTENT", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "3"))
        self.assertTrue(has_tree_badge(self.session.screen.text, "new.txt", "?"), self.session.screen.text)

    def test_deleting_the_selected_file_returns_to_the_root(self):
        # Given
        self.session.send("nn")
        self.session.wait_selection("notes.txt")
        # When
        (self.root / "notes.txt").unlink()
        # Then
        self.session.wait_for(lambda text: has_selection(text, "All changes") and "notes.txt" not in text,
                              "root selection without the deleted file", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "1"))
        self.assertIn("working", self.session.screen.text)

    def test_tracked_deletion_refreshes_the_diff_status(self):
        # Given
        path = self.root / "src/main.rs"
        # When
        path.unlink()
        # Then
        self.session.wait_for(
            lambda text: any("src/main.rs" in row and "Deleted" in row for row in text.splitlines()),
            "deleted file header",
            timeout=4,
        )
        self.assertTrue(has_tree_badge(self.session.screen.text, "main.rs", "DM"), self.session.screen.text)

    def test_staging_updates_badges_and_removes_the_unstaged_section(self):
        # Given
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.git("add", "src/main.rs")
        # Then
        self.session.wait_for(lambda text: has_tree_badge_color(self.session.screen, "main.rs", "M", (124, 181, 141)) and " UNSTAGED " not in text and "working" in text,
                              "staged content without an unstaged section", timeout=4)
        self.assertIn(" STAGED ", self.session.screen.text)

    def test_unstaging_updates_badges_and_removes_the_staged_section(self):
        # Given
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.git("restore", "--staged", "src/main.rs")
        # Then
        self.session.wait_for(lambda text: has_tree_badge_color(self.session.screen, "main.rs", "M", (229, 143, 150)) and " STAGED " not in text and "working" in text,
                              "unstaged content without a staged section", timeout=4)
        self.assertFalse(has_tree_badge(self.session.screen.text, "main.rs", "MM"), self.session.screen.text)

    def test_commit_clears_both_panes(self):
        # Given
        self.git("add", ".")
        self.session.wait_for(lambda text: " UNSTAGED " not in text and " UNTRACKED " not in text,
                              "staged changes", timeout=4)
        # When
        self.git("commit", "-m", "all changes")
        # Then
        self.session.wait("No changes in this view", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "All changes", "0"))

    def test_staged_rename_replaces_the_tree_path(self):
        # Given
        self.git("restore", "--source=HEAD", "--staged", "--worktree", "src/main.rs")
        self.session.wait_for(lambda text: "main.rs" not in text,
                              "restored file", timeout=4)
        # When
        self.git("mv", "src/main.rs", "src/renamed.rs")
        # Then
        self.session.wait_for(lambda text: has_tree_badge(text, "renamed.rs", "R"), "renamed tree row", timeout=4)
        self.session.wait("src/renamed.rs", timeout=4)
        self.assertFalse(has_tree_badge(self.session.screen.text, "main.rs", "M"), self.session.screen.text)

    def test_branch_switch_refreshes_the_diff(self):
        # Given
        branch = "watch-new-branch"
        # The target branch contains the staged version; working edits remain.
        tree = self.git("write-tree").stdout.decode().strip()
        commit = self.git("commit-tree", tree, "-p", "HEAD", "-m", "staged branch").stdout.decode().strip()
        self.git("branch", branch, commit)
        # When
        self.git("checkout", branch)
        # Then
        self.session.wait_for(lambda text: has_tree_badge(text, "main.rs", "M") and " STAGED " not in text and "working" in text,
                              "diff against the new branch", timeout=4)
        self.assertNotIn(branch, self.session.screen.text)

    def test_ignore_change_removes_a_file_from_both_panes(self):
        # Given
        ignore = self.root / ".gitignore"
        # When
        ignore.write_text("notes.txt\n")
        # Then
        self.session.wait_for(lambda text: has_tree_badge(text, ".gitignore", "?") and "untracked note" not in text,
                              "updated ignore rules", timeout=4)
        self.assertFalse(has_tree_badge(self.session.screen.text, "notes.txt", "?"), self.session.screen.text)

    def test_manual_refresh_detects_an_external_ignore_change(self):
        # Given
        ignore = self.external_ignore(reconcile=3600)
        ignore.write_text("")
        # When
        self.session.send("r")
        # Then
        self.session.wait("EXTERNAL_CHANGE", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "external.txt", "?"), self.session.screen.text)

    def test_periodic_scan_recovers_an_undelivered_content_change(self):
        # Given
        target = self.root / "notes.txt"
        self.restart_session(reconcile=1)
        # mmap writes can remain invisible to native watchers until close-write.
        with target.open("r+b") as handle, mmap.mmap(handle.fileno(), 0) as mapped:
            # When
            mapped[:] = b"PERIODIC_EDIT!\n"
            mapped.flush()
            # Then
            self.session.wait("PERIODIC_EDIT!", timeout=4)
            self.assertTrue(has_tree_badge(self.session.screen.text, "notes.txt", "?"), self.session.screen.text)

    def test_external_ignore_replacement_refreshes_without_periodic_scan(self):
        # Given
        ignore = self.external_ignore(reconcile=3600)
        replacement = ignore.with_name("replacement")
        replacement.write_text("")
        # When
        replacement.replace(ignore)
        # Then
        self.session.wait("EXTERNAL_CHANGE", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "external.txt", "?"), self.session.screen.text)

    def test_nested_ignore_negation_updates_both_panes(self):
        # Given
        (self.root / ".gitignore").write_text("*.log\n")
        nested = self.root / "nested"
        nested.mkdir()
        (nested / "keep.log").write_text("NESTED_REVEALED\n")
        self.restart_session()
        # When
        (nested / ".gitignore").write_text("!keep.log\n")
        # Then
        self.session.wait("NESTED_REVEALED", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "keep.log", "?"), self.session.screen.text)

    def test_linked_worktree_observes_its_external_index(self):
        # Given
        directory = tempfile.TemporaryDirectory(prefix="dift-linked-")
        self.addCleanup(directory.cleanup)
        linked = Path(directory.name) / "worktree"
        self.git("worktree", "add", "-b", "linked", str(linked))
        (linked / "src/main.rs").write_text("LINKED_CONTENT\n")
        self.restart_session(cwd=linked)
        # When
        self.git("add", "src/main.rs", cwd=linked)
        # Then
        self.session.wait_for(lambda text: has_tree_badge(text, "main.rs", "M") and " UNSTAGED " not in text,
                              "linked worktree index update", timeout=4)
        self.assertIn("LINKED_CONTENT", self.session.screen.text)

    def test_refresh_preserves_filter_selection_and_updates_search_matches(self):
        # Given
        self.session.resize(160, 35)
        self.session.send("nfsrc\r/working\r")
        self.session.wait("/working: 1 matches")
        # When
        (self.root / "src/main.rs").write_text("working\nworking again\n")
        # Then
        self.session.wait("/working: 2 matches", timeout=4)
        self.assertTrue(has_selection(self.session.screen.text, "main.rs"))
        self.assertIn("filter: src", self.session.screen.text)
        self.assertNotIn("notes.txt", self.session.screen.text)

    def test_write_burst_eventually_displays_the_last_content(self):
        # Given
        path = self.root / "src/main.rs"
        writer = threading.Thread(target=self.write_burst, args=(path,))
        self.addCleanup(writer.join)
        # When
        writer.start()
        # Then
        self.session.wait("BURST_FINAL", timeout=4)
        self.assertTrue(has_tree_badge(self.session.screen.text, "main.rs", "MM"), self.session.screen.text)

    def test_renaming_a_directory_updates_its_children_in_both_panes(self):
        # Given
        directory = self.root / "draft"
        directory.mkdir()
        (directory / "child.txt").write_text("DIRECTORY_CONTENT\n")
        self.restart_session()
        # When
        directory.rename(self.root / "renamed")
        # Then
        self.session.wait_for(lambda text: "renamed/child.txt" in text and "draft" not in text,
                              "renamed directory and diff path", timeout=4)
        self.assertIn("DIRECTORY_CONTENT", self.session.screen.text)

    def test_editing_inside_a_renamed_directory_is_still_observed(self):
        # Given
        directory = self.root / "draft"
        directory.mkdir()
        (directory / "child.txt").write_text("DIRECTORY_CONTENT\n")
        self.restart_session()
        directory.rename(self.root / "renamed")
        self.session.wait_for(lambda text: "renamed/child.txt" in text and "draft" not in text,
                              "renamed directory", timeout=4)
        # When
        (self.root / "renamed/child.txt").write_text("AFTER_RENAME\n")
        # Then
        self.session.wait("AFTER_RENAME", timeout=4)
        self.assertIn("renamed/child.txt", self.session.screen.text)

    def test_edit_during_syntax_enrichment_keeps_the_latest_content(self):
        # Given
        path, source = self.syntax_fixture()
        path.write_text(source + 'export const last = "INTERMEDIATE";\n')
        self.session.wait("INTERMEDIATE", timeout=4)
        self.session.wait("Rendering", timeout=4)
        # When
        path.write_text(source + 'export const last = "FINAL_VALUE";\n')
        # Then
        self.session.wait("FINAL_VALUE", timeout=4)
        self.session.wait("Watching", timeout=10)
        self.assertIn("FINAL_VALUE", self.session.screen.text)
        self.assertNotIn("INTERMEDIATE", self.session.screen.text)

    def test_resize_and_exit_restore_terminal(self):
        self.session.resize(80, 25)
        self.session.wait_selection("All changes")
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)
        self.assertIn(b"\x1b[?1049l", self.session.raw)

    def fake_editor(self, exit_code=0):
        script = self.root / ".git/fake editor.py"
        script.write_text('''import json
import os
from pathlib import Path
import sys
import termios

target = Path(sys.argv[-1].rsplit(":", 2)[0] if "--goto" in sys.argv else sys.argv[-1])
record = Path(__file__).with_name("editor-record.json")
previous = json.loads(record.read_text()) if record.exists() else []
previous.append({
    "argv": sys.argv[1:],
    "cwd": os.getcwd(),
    "contents": target.read_text(),
    "canonical": bool(termios.tcgetattr(0)[3] & termios.ICANON),
    "echo": bool(termios.tcgetattr(0)[3] & termios.ECHO),
})
record.write_text(json.dumps(previous))
print("\\033[H\\033[2JEDITOR_READY " + str(len(previous)), flush=True)
response = sys.stdin.readline().strip()
if response == "save":
    target.write_text("EDITOR_SAVED_CONTENT\\n")
sys.exit(int(sys.argv[1]))
''')
        return shlex.join([sys.executable, str(script), str(exit_code), "argument with spaces"])

    def editor_record(self):
        return json.loads((self.root / ".git/editor-record.json").read_text())[-1]

    def supported_editor(self, name):
        self.fake_editor()
        directory = self.root / ".git/editor tools"
        directory.mkdir(exist_ok=True)
        wrapper = directory / name
        command = shlex.join([sys.executable, str(self.root / ".git/fake editor.py"),
                              "0", "argument with spaces"])
        wrapper.write_text(f'#!/bin/sh\nexec {command} "$@"\n')
        wrapper.chmod(0o755)
        return shlex.quote(str(wrapper))

    def test_nvim_editor_receives_a_line_before_the_argument_delimiter(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": self.supported_editor("nvim") + " --clean --", "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", "--clean", "+1", "--", str(self.root / "src/main.rs")])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_helix_editor_receives_a_line_and_preserves_literal_arguments(self):
        # Given
        self.session.stop()
        command = self.supported_editor("hx") + " --config 'literal $value; text'"
        self.start_session(env={"VISUAL": command, "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", "--config", "literal $value; text", "+1", str(self.root / "src/main.rs")])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_vscode_editor_receives_goto_and_preserves_wait(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": self.supported_editor("code") + " --wait", "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", "--wait", "--goto", str(self.root / "src/main.rs") + ":1:1"])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_uppercase_o_opens_a_supported_editor_without_positioning(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": self.supported_editor("nvim"), "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("O")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", str(self.root / "src/main.rs")])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_editor_shell_wrappers_keep_normal_opening(self):
        # Given
        self.session.stop()
        command = "env " + self.supported_editor("nvim")
        self.start_session(env={"VISUAL": command, "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", str(self.root / "src/main.rs")])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_editor_shell_expansions_keep_normal_opening(self):
        # Given
        self.session.stop()
        command = self.supported_editor("nvim") + ' "$DIFT_TEST_EDITOR_ARGUMENT"'
        self.start_session(env={"VISUAL": command, "EDITOR": None,
                                "DIFT_TEST_EDITOR_ARGUMENT": "expanded argument"})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", "expanded argument", str(self.root / "src/main.rs")])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_editor_position_tracks_the_visible_hunk_after_scrolling(self):
        # Given
        self.session.stop()
        path = self.root / "src/main.rs"
        lines = [f"original line {number}\n" for number in range(1, 81)]
        path.write_text("".join(lines))
        self.git("add", "src/main.rs")
        self.git("commit", "-m", "many lines")
        lines[9] = "FIRST_CHANGE\n"
        lines[49] = "SECOND_CHANGE\n"
        path.write_text("".join(lines))
        self.start_session(env={"VISUAL": self.supported_editor("nvim"), "EDITOR": None})
        self.session.resize(100, 12)
        self.session.send("n\t\x1b[F")
        self.session.wait("SECOND_CHANGE")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"],
                         ["0", "argument with spaces", "+50", str(path)])
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_editor_receives_the_working_file_and_refreshes_after_saving(self):
        # Given
        self.session.stop()
        command = self.fake_editor()
        self.start_session(env={"VISUAL": command, "EDITOR": "missing-fallback-editor"})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        self.session.wait("EDITOR_READY 1")
        record = self.editor_record()
        self.session.send("save\n")
        # Then
        self.session.wait("EDITOR_SAVED_CONTENT")
        self.session.wait("Watching")
        self.session.wait_selection("main.rs")
        self.assertEqual(record["argv"], ["0", "argument with spaces", str(self.root / "src/main.rs")])
        self.assertEqual(record["cwd"], str(self.root))
        self.assertIn('"working"', record["contents"])
        self.assertTrue(record["canonical"])
        self.assertTrue(record["echo"])
        self.assertFalse(termios.tcgetattr(self.session.slave)[3] & termios.ICANON)
        self.assertIn(b"\x1b[?2004l", self.session.raw)
        self.assertIn(b"\x1b[?1049l", self.session.raw)
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)

    def test_blank_visual_falls_back_to_editor(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": " \t ", "EDITOR": self.fake_editor()})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"][-1], str(self.root / "src/main.rs"))
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_unset_visual_falls_back_to_editor(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": None, "EDITOR": self.fake_editor()})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"][-1], str(self.root / "src/main.rs"))
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_missing_editor_configuration_shows_a_hint(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": None, "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("Set VISUAL or EDITOR to open files")
        self.assertFalse(termios.tcgetattr(self.session.slave)[3] & termios.ICANON)

    def test_editor_launch_failure_returns_to_dift_without_falling_back(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": str(self.root / ".git/missing-editor"),
                                "EDITOR": self.fake_editor()})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("Cannot start $VISUAL")
        self.assertFalse((self.root / ".git/editor-record.json").exists())
        self.assertFalse(termios.tcgetattr(self.session.slave)[3] & termios.ICANON)
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)

    def test_unsuccessful_editor_exit_restores_the_view(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": self.fake_editor(exit_code=7), "EDITOR": None})
        self.session.send("no")
        self.session.wait("EDITOR_READY 1")
        # When
        self.session.send("close\n")
        # Then
        self.session.wait("$VISUAL exited unsuccessfully")
        self.session.wait_selection("main.rs")
        self.assertFalse(termios.tcgetattr(self.session.slave)[3] & termios.ICANON)
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)

    def test_editor_paths_preserve_spaces_and_shell_metacharacters(self):
        # Given
        self.session.stop()
        path = self.root / "src/-odd '$(touch INJECTED)' ; name.rs"
        path.write_text("SPECIAL_PATH_CONTENT\n")
        self.start_session(env={"VISUAL": self.supported_editor("nvim"), "EDITOR": None})
        self.session.send("f-odd\rn")
        self.session.wait("SPECIAL_PATH_CONTENT")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertEqual(self.editor_record()["argv"][-1], str(path))
        self.assertFalse((self.root / "INJECTED").exists())
        self.session.send("close\n")
        self.session.wait("Watching")

    def test_repeated_editor_launches_survive_a_resize(self):
        # Given
        self.session.stop()
        self.start_session(env={"VISUAL": self.fake_editor(), "EDITOR": None})
        self.session.send("no")
        self.session.wait("EDITOR_READY 1")
        self.session.send("close\n")
        self.session.wait("Watching")
        # When
        self.session.send("o")
        self.session.wait("EDITOR_READY 2")
        self.session.resize(80, 25)
        self.session.send("close\n")
        # Then
        self.session.wait_selection("main.rs")
        self.session.wait("Watching")
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)

    def test_comparison_editor_opens_local_contents(self):
        # Given
        self.session.stop()
        self.git("commit", "-m", "staged version")
        self.start_session(env={"VISUAL": self.fake_editor(), "EDITOR": None},
                           args=("diff", "HEAD~1", "HEAD"))
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("EDITOR_READY 1")
        self.assertIn('"working"', self.editor_record()["contents"])
        self.session.send("save\n")
        self.session.wait("Snapshot")
        self.assertNotIn("EDITOR_SAVED_CONTENT", self.session.screen.text)

    def test_deleted_files_show_a_notice_without_starting_the_editor(self):
        # Given
        self.session.stop()
        (self.root / "src/main.rs").unlink()
        self.start_session(env={"VISUAL": self.fake_editor(), "EDITOR": None})
        self.session.send("n")
        self.session.wait_selection("main.rs")
        # When
        self.session.send("o")
        # Then
        self.session.wait("Cannot open")
        self.assertFalse((self.root / ".git/editor-record.json").exists())

    def test_idle_view_does_not_repeatedly_write_terminal_frames(self):
        self.session.advance(0.1)
        size = len(self.session.raw)
        self.session.advance(0.2)
        self.assertEqual(len(self.session.raw), size)

    def test_navigation_cancels_large_file_enrichment(self):
        (self.root / "a").mkdir()
        source = "".join(f"export const item_{i}: Record<string, number> = {{ count: {i} }};\n" for i in range(10000))
        large = self.root / "a/large.ts"
        large.write_text(source + 'export const last = "before";\n')
        self.git("add", "a/large.ts")
        self.git("commit", "-m", "large file")
        large.write_text(source + 'export const last = "LATE_VALUE";\n')
        self.session.wait("LATE_VALUE", timeout=4)
        # Text can arrive before the footer, or during a watch-coverage rescan.
        self.session.wait("Rendering", timeout=4)
        self.session.send("n")
        self.session.wait_selection("large.ts", timeout=3)
        self.session.advance(0.2)
        self.session.send("n")
        self.session.wait_selection("main.rs", timeout=3)
        self.session.wait("Watching", timeout=3)
        self.assertNotIn("LATE_VALUE", self.session.screen.text)


if __name__ == "__main__":
    unittest.main()
