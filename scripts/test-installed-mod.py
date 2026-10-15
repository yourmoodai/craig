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
import re
import sys
from release.bridge_check import request as bridge_request

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
        for key in ('SYSTEMROOT','WINDIR','COMSPEC','TEMP','TMP'):
            if key in os.environ: env[key]=os.environ[key]
        env['USERPROFILE']=str(home)
        version = subprocess.run([binary, "--version"], cwd=workspace, env=env, text=True, capture_output=True, check=True, timeout=10)
        if not version.stdout.startswith("2.1.290 "):
            server.shutdown()
            raise SystemExit("installed UI fixture requires claude 2.1.290")
        terminal = Terminal([binary, "--plugin-dir", str(root), "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--setting-sources", "", "--tools", "", "--model", "claude-sonnet-4-5", "--system-prompt", "Respond with synthetic response."], workspace, env)
        output = b""
        try:
            stage='ready'
            print('installed stage: ready',flush=True)
            deadline=time.monotonic()+45
            submitted_at=None
            while time.monotonic()<deadline:
                output=(output+terminal.read())[-65536:]  # 64 KiB, held only in memory
                if not terminal.alive():
                    raise RuntimeError(f'{stage}: installed_host_exited')
                if any(marker in output for marker in [b'oauth/authorize',b'Paste code',b'Login successful']):
                    raise RuntimeError(f'{stage}: unexpected_authentication')
                visible=re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]',b'',output)
                if stage=='ready' and b'trust' in visible and b'No,' in visible:
                    terminal.write(b'\x1b[B\r');output=b''
                elif stage=='ready' and '❯'.encode() in visible:
                    terminal.write(b'/craig')
                    stage='command';output=b'';deadline=time.monotonic()+10
                elif stage=='command' and b'/craig' in visible:
                    terminal.write(b'\r')
                    print('installed stage: pane',flush=True)
                    stage='pane';output=b'';deadline=time.monotonic()+30
                elif stage=='pane' and b'personal metrics' in visible:
                    terminal.write(b'm')
                    print('installed stage: metrics',flush=True)
                    stage='metrics';output=b'';deadline=time.monotonic()+30
                elif stage=='metrics' and b'enable live metrics and sampled local judge' in visible:
                    terminal.write(b'l')
                    print('installed stage: consent',flush=True)
                    stage='consent';output=b'';deadline=time.monotonic()+30
                elif stage=='consent' and b'disable live metrics' in visible:
                    state=bridge_request(['node',str(root/'bridge/review.mjs')],{'command':'status'},env,stage)
                    if state['consent']['live'] is not True or state['consent']['history'] is not False:
                        raise RuntimeError('consent: ui_worker_mismatch')
                    terminal.write(b'\x1b')
                    time.sleep(0.2)
                    terminal.write(original.encode()+b'\r')
                    submitted_at=time.monotonic()
                    print('installed stage: submission',flush=True)
                    stage='submission';output=b'';deadline=time.monotonic()+60
                elif stage=='submission' and any(replacement in json.dumps(body) for body in received):
                    if any(original in json.dumps(body) for body in received):
                        raise RuntimeError('submission: original_reached_provider')
                    submission_ms=(time.monotonic()-submitted_at)*1000
                    print('installed stage: metrics_recorded',flush=True)
                    stage='metrics_recorded';deadline=time.monotonic()+15
                elif stage=='metrics_recorded':
                    metrics=bridge_request(['node',str(root/'bridge/review.mjs')],{'command':'status'},env,stage)
                    if metrics['consent']['live'] is not True or metrics['consent']['history'] is not False:
                        raise RuntimeError('metrics_recorded: unexpected_consent')
                    if metrics['summary']['live']['observations']!=1:
                        continue
                    assert metrics['summary']['live']['removed_characters']==5
                    assert original.encode() not in (profile/'craig/metrics.sqlite3').read_bytes()
                    assert original not in json.dumps(metrics)
                    print(json.dumps({'detector':env['CRAIG_DETECTOR'],'submission_to_provider_ms':submission_ms,'samples':1}))
                    print('installed mod: UI opt-in, native assessment, text-free metrics and sanitized provider payload passed')
                    return
            # Host startup can include account information; do not persist terminal output.
            markers=[word for word in ['trust','Yes','No','API','key','Enter','continue','Welcome','Terms','theme','danger','bypass','Press','shortcuts','permissions','personal metrics','local filter ready'] if word.encode() in visible]
            raise RuntimeError(f'{stage}: installed_host_timeout; observed_ui_markers='+','.join(markers))
        finally:
            failed=sys.exc_info()[0] is not None
            terminal.close()
            try:
                bridge_request(['node',str(root/'bridge/review.mjs')],{'command':'shutdown'},env,'cleanup')
            except Exception:
                if not failed: raise
            finally:
                server.shutdown()
                server.server_close()



if __name__ == "__main__":
    main()
