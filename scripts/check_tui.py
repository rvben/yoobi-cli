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

    try:
        pump(0.8)
        assert "YOOBI" in text(), text()
        assert "DEMO" in text()
        send(b"?")
        assert "Keyboard guide" in text()
        assert "q / Ctrl-C" in text()
        assert "Refresh timesheet" in text()
        send(b"\x1b")
        send(b"/never\r")
        assert "No dates match" in text()
        assert "Date filter" in text()
        send(b"\x1b")
        send(b"]")
        assert "2026-12" in text(), text()
        send(b"[")
        assert "2026-11" in text()
        send(b"/2026-11-11\r")
        send(b"\r")
        assert "Booked" in text(), text()
        assert "8h00" in text()
        assert "Actual" not in text()
        assert "Compare with actual work" not in text()
        send(b"\x1b[F")
        assert "State" in text(), text()
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
