"""Check the real terminal adapter. Run after `cargo build` on Unix.

Uses only the Python standard library. Does not contact external providers.
"""

import errno
import fcntl
import http.server
import os
import pty
import re
import select
import signal
import struct
import tempfile
import termios
import threading
import time
import unittest
from pathlib import Path

BINARY = Path(__file__).resolve().parents[1] / "target/debug/shell-ai"
ANSI = re.compile(rb"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[78]")


class StubProvider(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        body = b'{"choices":[{"message":{"content":"printf demo"}}]}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


class TerminalSmoke(unittest.TestCase):
    def test_editing_resize_editor_and_terminal_restore(self):
        self.check_editing()

    def test_editing_with_shell_command_capture(self):
        self.check_editing(capture_stdout=True)

    def test_question_mark_launches_the_generated_zsh_widget(self):
        self.check_editing(launch_key=True)

    def test_cursor_query_timeout_preserves_stdout_and_terminal_mode(self):
        self.check_editing(capture_stdout=True, query_timeout=True)

    def test_suggested_command_reaches_the_shell_capture_without_terminal_codes(self):
        self.check_editing(capture_stdout=True, submit=True)

    def check_editing(
        self, capture_stdout=False, launch_key=False, query_timeout=False, submit=False
    ):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            provider = None
            base_url = "http://127.0.0.1:1/v1"
            if submit:
                provider = http.server.HTTPServer(("127.0.0.1", 0), StubProvider)
                self.addCleanup(provider.server_close)
                base_url = f"http://127.0.0.1:{provider.server_port}/v1"
            config = root / "config.toml"
            config.write_text(
                '[defaults]\nprovider = "test"\nmodel = "test"\n'
                f'[providers.test]\nbase_url = "{base_url}"\n'
                'api_key = "TEST_KEY"\nmodels = ["test"]\n'
            )
            editor = root / "test editor"
            editor.write_text('#!/bin/sh\nprintf "from editor" > "$1"\n')
            editor.chmod(0o700)
            (root / "completion-folder").mkdir()
            env = dict(
                os.environ,
                SHELL_AI_CONFIG=str(config),
                XDG_STATE_HOME=str(root / "state"),
                TEST_KEY="unused",
                EDITOR=f"'{editor}'",
                TERM="xterm-256color",
                SHELL_AI_BIN=str(BINARY),
                SHELL_AI_SHELL="zsh",
                ZDOTDIR=str(root),
            )
            (root / ".zshrc").write_text(
                'PS1="SHELL> "\nRPROMPT=""\neval "$("$SHELL_AI_BIN" init zsh)"\n'
            )
            captured = root / "captured-stdout"
            pid, master = pty.fork()
            if pid == 0:
                os.chdir(root)
                if launch_key:
                    os.execve("/bin/zsh", ["zsh", "-di"], env)
                if capture_stdout:
                    os.execve(
                        "/bin/zsh",
                        [
                            "zsh",
                            "-dfc",
                            'result=$("$1" exec </dev/tty); code=$?; printf "%s" "$result" > "$2"; exit $code',
                            "zsh",
                            str(BINARY),
                            str(captured),
                        ],
                        env,
                    )
                os.execve(BINARY, [str(BINARY), "exec"], env)
            if provider:
                # Start threads only after forkpty, which must run single-threaded.
                thread = threading.Thread(target=provider.serve_forever, daemon=True)
                thread.start()
                self.addCleanup(thread.join)
                self.addCleanup(provider.shutdown)
            initial = termios.tcgetattr(master)
            self.addCleanup(os.close, master)
            exited = False
            output = bytearray()

            def resize(rows, columns):
                fcntl.ioctl(
                    master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0)
                )
                os.kill(pid, signal.SIGWINCH)

            def receive(expected):
                batch = bytearray()
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    if select.select([master], [], [], 0.1)[0]:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError as error:
                            if error.errno == errno.EIO:
                                break
                            raise
                        if not chunk:
                            break
                        batch.extend(chunk)
                        output.extend(chunk)
                        # The harness starts near the bottom with a shell prompt offset.
                        if b"\x1b[6n" in batch:
                            os.write(master, b"\x1b[5;5R")
                            batch[:] = batch.replace(b"\x1b[6n", b"")
                    elif expected in ANSI.sub(b"", batch):
                        # Wait for the complete redraw before the next key sequence.
                        return
                self.fail(
                    f"terminal did not show {expected!r}: {bytes(batch[-500:])!r}"
                )

            def send(keys, expected):
                os.write(master, keys)
                receive(expected)

            try:
                resize(6, 40)
                if not query_timeout:
                    if launch_key:
                        receive(b"SHELL> ")
                        os.write(master, b"?")
                    receive("AI› ".encode())
                    prompt = b"long prompt " * 8
                    send(b"\x1b[200~" + prompt + b"\x1b[201~", prompt)
                    self.assertRegex(bytes(output), rb"\x1b\[[1-9][0-9]*S")
                    send(b"\x1f", "AI› ".encode())
                    send(b"\x1br", prompt)
                    resize(8, 20)
                    receive(b"long prompt")
                    send(b"\x07", b"from editor")
                    send(b"\x1f", b"long prompt")
                    send(b"\x1br", b"from editor")
                    editor.write_text("#!/bin/sh\nexit 9\n")
                    send(b"\x07", b"EDITOR exited")
                    send(b"\x12", b"search history: ")
                    send(b"missing", b"missing")
                    send(b"\x1b", b"from editor")
                    send(b"\x1b", "AI› ".encode())
                    send(b"\x1b[200~completion-\x1b[201~", b"completion-")
                    # Give this completion enough columns to inspect the whole result.
                    resize(10, 60)
                    receive(b"completion-")
                    send(b"\t", b"completion-folder/")
                    os.write(master, b"\r" if submit else b"\x03")
                    if launch_key:
                        receive(b"SHELL> ")
                        os.write(master, b"exit\r")
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    # macOS can wait for pending PTY output before process exit.
                    if select.select([master], [], [], 0.05)[0]:
                        try:
                            output.extend(os.read(master, 65536))
                        except OSError as error:
                            if error.errno != errno.EIO:
                                raise
                    found, status = os.waitpid(pid, os.WNOHANG)
                    if found:
                        exited = True
                        self.assertEqual(
                            os.waitstatus_to_exitcode(status),
                            14 if query_timeout else 0,
                        )
                        break
                    time.sleep(0.05)
                self.assertTrue(
                    exited, f"session did not close: {ANSI.sub(b'', output[-2000:])!r}"
                )
                self.assertIn(b"\x1b[6n", output)
                if not query_timeout:
                    self.assertIn(b"\x1b[?2004l", output)
                if capture_stdout:
                    self.assertEqual(
                        captured.read_bytes(), b"printf demo" if submit else b""
                    )
                restored = termios.tcgetattr(master)
                for flag in (termios.ICANON, termios.ECHO, termios.ISIG):
                    self.assertEqual(restored[3] & flag, initial[3] & flag)
            finally:
                if not exited:
                    os.kill(pid, signal.SIGKILL)
                    while select.select([master], [], [], 0.1)[0]:
                        try:
                            if not os.read(master, 65536):
                                break
                        except OSError:
                            break
                    os.waitpid(pid, 0)


if __name__ == "__main__":
    unittest.main()
