"""Unix terminal integration checks; run after cargo build, using only Python's stdlib."""

import codecs
import errno
import fcntl
import mmap
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import threading
import unicodedata
import unittest


BINARY = Path(__file__).resolve().parents[1] / "target/debug/dift"


class Screen:
    """Small VT screen reader for the cursor/erase sequences emitted by Ratatui."""

    def __init__(self, width=120, height=35):
        self.width, self.height = width, height
        self.rows = [[" "] * width for _ in range(height)]
        self.x = self.y = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
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
                elif final == "K" and self.y < self.height:
                    start = 0 if args[0] in (1, 2) else self.x
                    end = self.width if args[0] in (0, 2) else self.x + 1
                    self.rows[self.y][start:end] = [" "] * (end - start)
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
                elif not unicodedata.category(ch).startswith("C"):
                    width = 0 if unicodedata.combining(ch) else (2 if unicodedata.east_asian_width(ch) in "WF" else 1)
                    if 0 <= self.y < self.height and 0 <= self.x < self.width:
                        self.rows[self.y][self.x] = ch
                    self.x += width

    @property
    def text(self):
        return "\n".join("".join(row) for row in self.rows)


class Session:
    def __init__(self, cwd, config):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.screen = Screen()
        self.resize(120, 35)
        self.pid = os.fork()
        if self.pid == 0:
            os.close(self.master)
            os.login_tty(self.slave)
            os.chdir(cwd)
            os.execve(str(BINARY), [str(BINARY), "--config", str(config)], {**os.environ, "TERM": "xterm-256color"})
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
        self.send("\x03")
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

    def start_session(self, cwd=None, reconcile=3600):
        # Event tests must complete long before the periodic fallback can run.
        config = self.root / ".git/terminal-config.kdl"
        config.write_text(f"watch {{ reconcile-seconds {reconcile}; }}\n")
        self.session = Session(cwd or self.root / "src", config)
        self.addCleanup(self.session.close)
        self.session.wait("Watching")

    def restart_session(self, cwd=None, reconcile=3600):
        self.session.stop()
        self.start_session(cwd, reconcile)

    def git(self, *args, cwd=None):
        return subprocess.run([
            "git", "-C", str(cwd or self.root), "-c", "user.name=Terminal Test",
            "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false",
            "-c", "core.hooksPath=/nonexistent-dift-hooks", *args
        ], check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

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
        self.assertIn("Changes · 0", self.session.screen.text)

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
        self.session.wait("src/main.rs · Split")
        self.assertIn("UNSTAGED", self.session.screen.text)
        self.session.send("n")
        self.session.wait("notes.txt · Split")
        self.session.send("p")
        self.session.wait("src/main.rs · Split")

    def test_filter_changes_both_tree_and_aggregate_diff(self):
        self.session.send("fnotes\r")
        self.session.wait("Changes · 1")
        self.session.wait("Watching")
        self.assertIn("untracked note", self.session.screen.text)
        self.assertNotIn("main.rs", self.session.screen.text)

    def test_search_is_literal_and_input_keeps_quit_character(self):
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
        self.assertIn("main.rs  US", self.session.screen.text)

    def test_submodule_head_change_refreshes_a_clean_tree(self):
        # Given
        self.add_submodule()
        # When
        self.git("commit", "--allow-empty", "-m", "new module commit", cwd=self.root / "module")
        # Then
        self.session.wait("Submodule change", timeout=4)
        self.assertIn("Changes · 1", self.session.screen.text)
        self.assertIn("module  U", self.session.screen.text)

    def test_atomic_save_refreshes_the_selected_file(self):
        # Given
        self.session.send("n")
        self.session.wait("src/main.rs · Unified")
        temporary = self.root / "src/save.tmp"
        temporary.write_text("ATOMIC_SAVE\n")
        # When
        os.replace(temporary, self.root / "src/main.rs")
        # Then
        self.session.wait("ATOMIC_SAVE", timeout=4)
        self.assertIn("src/main.rs · Unified", self.session.screen.text)

    def test_untracked_creation_updates_both_panes(self):
        # Given
        path = self.root / "new.txt"
        # When
        path.write_text("NEW_FILE_CONTENT\n")
        # Then
        self.session.wait("NEW_FILE_CONTENT", timeout=4)
        self.assertIn("Changes · 3", self.session.screen.text)
        self.assertIn("new.txt  ?", self.session.screen.text)

    def test_deleting_the_selected_file_returns_to_the_root(self):
        # Given
        self.session.send("nn")
        self.session.wait("notes.txt · Unified")
        # When
        (self.root / "notes.txt").unlink()
        # Then
        self.session.wait_for(lambda text: "Repository · Unified" in text and "notes.txt" not in text,
                              "root selection without the deleted file", timeout=4)
        self.assertIn("Changes · 1", self.session.screen.text)
        self.assertIn("working", self.session.screen.text)

    def test_tracked_deletion_refreshes_the_diff_status(self):
        # Given
        path = self.root / "src/main.rs"
        # When
        path.unlink()
        # Then
        self.session.wait(" D  src/main.rs", timeout=4)
        self.assertIn("main.rs  US", self.session.screen.text)

    def test_staging_updates_badges_and_removes_the_unstaged_section(self):
        # Given
        self.session.send("n")
        self.session.wait("src/main.rs · Unified")
        # When
        self.git("add", "src/main.rs")
        # Then
        self.session.wait_for(lambda text: "main.rs  S" in text and " UNSTAGED " not in text and "working" in text,
                              "staged content without an unstaged section", timeout=4)
        self.assertIn(" STAGED ", self.session.screen.text)

    def test_unstaging_updates_badges_and_removes_the_staged_section(self):
        # Given
        self.session.send("n")
        self.session.wait("src/main.rs · Unified")
        # When
        self.git("restore", "--staged", "src/main.rs")
        # Then
        self.session.wait_for(lambda text: "main.rs  U" in text and " STAGED " not in text and "working" in text,
                              "unstaged content without a staged section", timeout=4)
        self.assertNotIn("main.rs  US", self.session.screen.text)

    def test_commit_clears_both_panes(self):
        # Given
        self.git("add", ".")
        self.session.wait_for(lambda text: " UNSTAGED " not in text and " UNTRACKED " not in text,
                              "staged changes", timeout=4)
        # When
        self.git("commit", "-m", "all changes")
        # Then
        self.session.wait("No changes in this view", timeout=4)
        self.assertIn("Changes · 0", self.session.screen.text)

    def test_staged_rename_replaces_the_tree_path(self):
        # Given
        self.git("restore", "--source=HEAD", "--staged", "--worktree", "src/main.rs")
        self.session.wait_for(lambda text: "main.rs" not in text,
                              "restored file", timeout=4)
        # When
        self.git("mv", "src/main.rs", "src/renamed.rs")
        # Then
        self.session.wait("renamed.rs  S", timeout=4)
        self.session.wait("src/renamed.rs", timeout=4)
        self.assertNotIn("main.rs  S", self.session.screen.text)

    def test_branch_switch_refreshes_the_branch_label(self):
        # Given
        branch = "watch-new-branch"
        # When
        self.git("checkout", "-b", branch)
        # Then
        self.session.wait(branch, timeout=4)
        self.assertIn("working", self.session.screen.text)

    def test_ignore_change_removes_a_file_from_both_panes(self):
        # Given
        ignore = self.root / ".gitignore"
        # When
        ignore.write_text("notes.txt\n")
        # Then
        self.session.wait_for(lambda text: ".gitignore  ?" in text and "untracked note" not in text,
                              "updated ignore rules", timeout=4)
        self.assertNotIn("notes.txt  ?", self.session.screen.text)

    def test_manual_refresh_detects_an_external_ignore_change(self):
        # Given
        ignore = self.external_ignore(reconcile=3600)
        ignore.write_text("")
        # When
        self.session.send("r")
        # Then
        self.session.wait("EXTERNAL_CHANGE", timeout=4)
        self.assertIn("external.txt  ?", self.session.screen.text)

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
            self.assertIn("notes.txt  ?", self.session.screen.text)

    def test_external_ignore_replacement_refreshes_without_periodic_scan(self):
        # Given
        ignore = self.external_ignore(reconcile=3600)
        replacement = ignore.with_name("replacement")
        replacement.write_text("")
        # When
        replacement.replace(ignore)
        # Then
        self.session.wait("EXTERNAL_CHANGE", timeout=4)
        self.assertIn("external.txt  ?", self.session.screen.text)

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
        self.assertIn("keep.log  ?", self.session.screen.text)

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
        self.session.wait_for(lambda text: "main.rs  S" in text and " UNSTAGED " not in text,
                              "linked worktree index update", timeout=4)
        self.assertIn("LINKED_CONTENT", self.session.screen.text)

    def test_refresh_preserves_filter_selection_and_updates_search_matches(self):
        # Given
        self.session.send("nfsrc\r/working\r")
        self.session.wait("/working: 1 matches")
        # When
        (self.root / "src/main.rs").write_text("working\nworking again\n")
        # Then
        self.session.wait("/working: 2 matches", timeout=4)
        self.assertIn("src/main.rs · Unified", self.session.screen.text)
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
        self.assertIn("main.rs  US", self.session.screen.text)

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
        self.assertIn("Rendering", self.session.screen.text)
        # When
        path.write_text(source + 'export const last = "FINAL_VALUE";\n')
        # Then
        self.session.wait("FINAL_VALUE", timeout=4)
        self.session.wait("Watching", timeout=10)
        self.assertIn("FINAL_VALUE", self.session.screen.text)
        self.assertNotIn("INTERMEDIATE", self.session.screen.text)

    def test_resize_and_exit_restore_terminal(self):
        self.session.resize(80, 25)
        self.session.wait("Repository · Unified")
        self.assertEqual(self.session.stop(), 0)
        self.assertEqual(termios.tcgetattr(self.session.slave), self.session.original)
        self.assertIn(b"\x1b[?1049l", self.session.raw)

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
        self.assertIn("Rendering", self.session.screen.text)
        self.session.send("n")
        self.session.wait("a/large.ts · Unified", timeout=3)
        self.session.advance(0.2)
        self.session.send("n")
        self.session.wait("src/main.rs · Unified", timeout=3)
        self.session.wait("Watching", timeout=3)
        self.assertNotIn("LATE_VALUE", self.session.screen.text)


if __name__ == "__main__":
    unittest.main()
