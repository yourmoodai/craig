#!/usr/bin/env python3
"""Exercise the installed composer against a synthetic loopback provider."""
import http.server
import json
import os
from pathlib import Path
import select
import shutil
import struct
import subprocess
import tempfile
import threading
import time
import queue

class Terminal:
    def __init__(self, argv, cwd, env):
        self.messages = queue.Queue()
        if os.name == 'nt':
            from winpty import PtyProcess
            self.process = PtyProcess.spawn(argv, cwd=str(cwd), env=env, dimensions=(50,160))
            self.master = None
        else:
            import fcntl, pty, termios
            self.master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',50,160,0,0))
            self.process = subprocess.Popen(argv,cwd=cwd,env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
            os.close(slave)
        def read():
            try:
                while True:
                    data = self.process.read(65536).encode() if self.master is None else os.read(self.master,65536)
                    if not data: break
                    self.messages.put(data)
            except (OSError,EOFError): pass
        threading.Thread(target=read,daemon=True).start()
    def read(self):
        try: return self.messages.get(timeout=0.1)
        except queue.Empty: return b''
    def write(self,data):
        if self.master is None: self.process.write(data.decode())
        else: os.write(self.master,data)
    def alive(self):
        return self.process.isalive() if self.master is None else self.process.poll() is None
    def close(self):
        if self.master is None: self.process.close(force=True)
        else:
            os.close(self.master)
            if self.process.poll() is None:
                self.process.terminate()
                try: self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill();self.process.wait(timeout=3)

root = Path(os.environ.get("CRAIG_PLUGIN_ROOT", Path(__file__).resolve().parent.parent)).resolve()
original = "You idiot, do not delete the test."
replacement = "You ya ya ya, do not delete the test."
received = []


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        assert self.headers.get("x-api-key") == "synthetic-local-key"
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))))
        received.append(body)
        if self.path.startswith("/v1/messages/count_tokens"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"input_tokens":20}')
            return
        message = {"id": "msg_synthetic", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5", "content": [], "stop_reason": None, "stop_sequence": None, "usage": {"input_tokens": 20, "output_tokens": 1}}
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        events = [
            {"type": "message_start", "message": message},
            {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
            {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "synthetic response"}},
            {"type": "content_block_stop", "index": 0},
            {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 2}},
            {"type": "message_stop"},
        ]
        for event in events:
            self.wfile.write(("event: " + event["type"] + "\ndata: " + json.dumps(event) + "\n\n").encode())
        self.wfile.flush()


def main():
    binary = os.environ.get("CRAIG_CLAUDE_BINARY") or shutil.which("claude")
    model = os.environ.get("CRAIG_MODEL_DIR")
    if not binary or not model:
        raise SystemExit("claude and CRAIG_MODEL_DIR are required")
    with tempfile.TemporaryDirectory(prefix="craig-host-") as directory:
        home = Path(directory).resolve()
        profile = home / ".claude"
        profile.mkdir()
        workspace = home / "workspace"
        workspace.mkdir()
        config = {"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 1, "projects": {str(workspace): {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}}, "customApiKeyResponses": {"approved": ["synthetic-local-key"], "rejected": []}}
        (home / ".claude.json").write_text(json.dumps(config))
        (profile / ".claude.json").write_text(json.dumps(config))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        env = {"PATH": os.environ["PATH"], "HOME": str(home), "CLAUDE_CONFIG_DIR": str(profile), "TERM": "xterm-256color", "ANTHROPIC_API_KEY": "synthetic-local-key", "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{server.server_port}", "CRAIG_MODEL_DIR": str(Path(model).resolve()), "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1", "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "ENABLE_CLAUDEAI_MCP_SERVERS": "false", "OTEL_SDK_DISABLED": "true"}
        env["CRAIG_DETECTOR"] = os.environ.get("CRAIG_DETECTOR", "bert")
        version = subprocess.run([binary, "--version"], cwd=workspace, env=env, text=True, capture_output=True, check=True)
        if not version.stdout.startswith("2.1.290 "):
            server.shutdown()
            raise SystemExit("installed UI fixture requires claude 2.1.290")
        for key in ('SYSTEMROOT','WINDIR','COMSPEC','TEMP','TMP'):
            if key in os.environ: env[key]=os.environ[key]
        env['USERPROFILE']=str(home)
        terminal = Terminal([binary, "--plugin-dir", str(root), "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--setting-sources", "", "--tools", "", "--model", "claude-sonnet-4-5", "--system-prompt", "Respond with synthetic response."], workspace, env)
        output = b""
        try:
            started = time.monotonic()
            sent = False
            submitted_at = None
            trusted = False
            pane_open = False
            metrics_open = False
            live_enabled = False
            while time.monotonic() - started < 45:
                output += terminal.read()
                if not terminal.alive():
                    # Report known diagnostic categories, never raw terminal content.
                    markers = (b"unknown option", b"Invalid API key", b"Cannot find", b"not found", b"ENOENT", b"SyntaxError", b"error", b"Error")
                    categories = [marker.decode() for marker in markers if marker in output]
                    raise SystemExit("installed host exited before submission: " + ", ".join(categories))
                if any(marker in output for marker in [b"oauth/authorize", b"Paste code", b"Login successful"]):
                    raise SystemExit("installed mod requires authentication; composer not tested")
                if not trusted and b"trust" in output and b"No," in output:
                    terminal.write(b"\x1b[B\r")
                    trusted = True
                    output = b""
                    started = time.monotonic()
                if not pane_open and time.monotonic() - started > 4:
                    terminal.write(b"/craig")
                    time.sleep(0.2)
                    terminal.write(b"\r")
                    pane_open = True
                if pane_open and not metrics_open and time.monotonic() - started > 6:
                    terminal.write(b"\x1b[<0;105;3M\x1b[<0;105;3m")
                    metrics_open = True
                if metrics_open and not live_enabled and time.monotonic() - started > 8:
                    terminal.write(b"\x1b[<0;110;6M\x1b[<0;110;6m")
                    live_enabled = True
                if not sent and time.monotonic() - started > 11:
                    terminal.write(b"\x1b")
                    time.sleep(0.2)
                    terminal.write(original.encode())
                    time.sleep(0.2)
                    terminal.write(b"\r")
                    submitted_at = time.monotonic()
                    sent = True
                if any(replacement in json.dumps(body) for body in received):
                    submission_ms = (time.monotonic() - submitted_at) * 1000
                    assert all(original not in json.dumps(body) for body in received)
                    result = subprocess.run(["node", str(root / "bridge/review.mjs")], input=json.dumps({"command": "status"}), text=True, capture_output=True, env=env, check=True)
                    metrics = json.loads(result.stdout)["value"]
                    assert metrics["consent"]["live"] is True
                    assert metrics["consent"]["history"] is False
                    assert metrics["summary"]["live"]["observations"] == 1
                    assert metrics["summary"]["live"]["removed_characters"] == 5
                    assert original.encode() not in (profile / "craig/metrics.sqlite3").read_bytes()
                    assert original not in result.stdout
                    print(json.dumps({"detector":env["CRAIG_DETECTOR"],"submission_to_provider_ms":submission_ms,"samples":1}))
                    print("installed mod: UI opt-in, native assessment, text-free metrics and sanitized provider payload passed")
                    return
            # Host startup can include account information; do not persist terminal output.
            raise SystemExit("installed mod did not forward the expected replacement")
        finally:
            terminal.close()
            subprocess.run(["node", str(root / "bridge/review.mjs")], input=json.dumps({"command": "shutdown"}), text=True, capture_output=True, env=env, timeout=20)
            server.shutdown()


if __name__ == "__main__":
    main()
