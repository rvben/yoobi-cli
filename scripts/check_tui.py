"""Exercise the real TUI in a PTY. Run with: uv run --no-project --with pyte scripts/check_tui.py."""

import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import termios
import time
from pathlib import Path

import pyte

binary = str(Path(__file__).resolve().parents[1] / "target/debug/yoobi-cli")
for width, height in [(120, 36), (40, 16)]:
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    before = termios.tcgetattr(slave)
    env = os.environ.copy()
    env["TERM"] = "xterm-256color"
    screen = pyte.Screen(width, height)
    stream = pyte.Stream(screen)
    proc = subprocess.Popen(
        [binary, "tui", "--demo", "--month", "2026-11"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        start_new_session=True,
    )

    def pump(seconds=0.3, master=master, stream=stream):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([master], [], [], max(0, end - time.monotonic()))[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                stream.feed(data.decode("utf-8", "replace"))

    def text(screen=screen):
        return "\n".join(screen.display)

    def send(keys, master=master, pump=pump):
        os.write(master, keys)
        pump()

    def expect(*phrases, absent=(), timeout=5):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            current = text()
            if all(p in current for p in phrases) and all(p not in current for p in absent):
                return
            pump(0.05)
        raise AssertionError(text())

    try:
        pump(0.8)
        expect("YOOBI", "DEMO")
        send(b"?")
        expect("Keyboard guide", "q / Ctrl-C", "Refresh timesheet")
        send(b"\x1b")
        expect(absent=("Keyboard guide",))
        send(b"/never\r")
        expect("No dates match", "Date filter")
        send(b"\x1b")
        expect(absent=("Date filter", "Find date"))
        send(b"]")
        expect("2026-12")
        send(b"[")
        expect("2026-11")
        send(b"/2026-11-11\r")
        expect("Date filter: 2026-11-11")
        send(b"\r")
        expect("Booked", "8h00", "Enter back")
        assert "Actual" not in text()
        assert "Compare with actual work" not in text()
        send(b"\x1b[F")
        expect("State")
        send(b"\x03")
        proc.wait(timeout=3)
        assert proc.returncode == 0
        after = termios.tcgetattr(slave)
        assert before == after, "terminal settings were not restored"
        print(
            json.dumps(
                {
                    "pty": f"{width}x{height}",
                    "help_search_months_detail_scroll_ctrl_c": "passed",
                    "terminal_restored": True,
                }
            )
        )
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        os.close(master)
        os.close(slave)
