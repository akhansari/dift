"""Read-only Unix startup benchmark of the real TUI, with Python's standard library.

Uses a 120x35 color terminal and built-in configuration for repeatable comparisons.
Reports warmed-filesystem timings; no caches are flushed. Syntax latency is the
first visible source row with distinct foreground colors, or null when the
opening viewport contains no such syntax (for example, a plain-text diff).
"""

import argparse
import json
from pathlib import Path
import statistics
import sys
import tempfile
import time

sys.dont_write_bytecode = True
import terminal_smoke


def visible_source(screen):
    # The default sidebar is 34 columns. The unified gutter is 14 columns:
    # two five-digit numbers followed by a diff sign and spaces.
    readable = highlighted = False
    for row, colors in zip(screen.rows[:-3], screen.foregrounds[:-3]):
        text = "".join(row[34:119])
        if (text[:5].strip().isdigit() or text[6:11].strip().isdigit()) and text[12] in " +-!":
            readable = True
            foregrounds = {
                color for char, color in zip(text[14:], colors[48:119])
                if not char.isspace()
            }
            highlighted |= len(foregrounds) > 1
    return readable, highlighted


def watch_count(pid):
    directory = Path(f"/proc/{pid}/fdinfo")
    if not directory.is_dir():
        return None
    count = 0
    for path in directory.iterdir():
        try:
            count += path.read_text().count("inotify wd:")
        except OSError:
            pass
    return count


def measure(repo, config, timeout):
    start = time.monotonic()
    session = terminal_smoke.Session(repo, config, env={"NO_COLOR": "", "COLORTERM": "truecolor"})
    result = dict(first_frame_ms=None, readable_diff_ms=None, highlighted_ms=None, complete_ms=None)
    try:
        while time.monotonic() - start < timeout:
            session.pump(0.01)
            elapsed = (time.monotonic() - start) * 1000
            text = session.screen.text
            if "All changes" in text and result["first_frame_ms"] is None:
                result["first_frame_ms"] = elapsed
            readable, highlighted = visible_source(session.screen)
            if readable and result["readable_diff_ms"] is None:
                result["readable_diff_ms"] = elapsed
            if highlighted and result["highlighted_ms"] is None:
                result["highlighted_ms"] = elapsed
            footer = text.splitlines()[-2].strip()
            fields = footer.split(" · ")
            if any(field.startswith("Error:") for field in fields):
                raise RuntimeError(footer)
            if "Watching" in fields or "Polling" in fields:
                result["complete_ms"] = elapsed
                result["native_watches"] = watch_count(session.pid)
                result["polling"] = "Polling" in fields
                return result
        raise TimeoutError(f"Startup exceeded {timeout}s; last screen:\n{session.screen.text}")
    finally:
        session.close()


def positive_int(value):
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path(__file__).resolve().parents[1] / "target/release/dift")
    parser.add_argument("--runs", type=positive_int, default=5)
    parser.add_argument("--timeout", type=positive_int, default=30)
    args = parser.parse_args()
    terminal_smoke.BINARY = args.binary.resolve(strict=True)
    repo = args.repo.resolve(strict=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="dift-startup-") as directory:
        config = Path(directory) / "config.kdl"
        config.write_text("")
        for run in range(1, args.runs + 1):
            result = measure(repo, config, args.timeout)
            results.append(result)
            print(json.dumps({"run": run, **result}), flush=True)
    medians = {}
    for key in ("first_frame_ms", "readable_diff_ms", "highlighted_ms", "complete_ms", "native_watches"):
        values = [result[key] for result in results if result[key] is not None]
        medians[key] = statistics.median(values) if values else None
    print(json.dumps({"median": medians}), flush=True)


if __name__ == "__main__":
    main()
