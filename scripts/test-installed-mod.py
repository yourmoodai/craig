#!/usr/bin/env python3
"""Exercise the installed host's prompt box against a synthetic loopback provider."""
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
import hashlib
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
            if self.process.poll() is None:
                self.process.terminate()
                try: self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.process.kill();self.process.wait(timeout=3)
            # Darwin can block close while the reader holds a pending PTY read.
            os.close(self.master)

root = Path(os.environ.get("CRAIG_PLUGIN_ROOT", Path(__file__).resolve().parent.parent)).resolve()
scenario = os.environ.get("CRAIG_HOST_SCENARIO", "success")
if scenario not in ('success', 'protected', 'missing_model', 'processing_timeout', 'worker_exit'):
    raise SystemExit('invalid host scenario')
original = "You `idiot`, do not delete the test." if scenario == 'protected' else "You idiot, do not delete the test."
strategy = os.environ.get("CRAIG_REPLACEMENT_STRATEGY", "fixed_phrase")
if strategy not in ("fixed_phrase", "omit"):
    raise SystemExit("invalid replacement strategy")
marker = "" if strategy == "omit" else os.environ.get("CRAIG_REPLACEMENT", "ya ya ya")
replacement = f"You {marker}, do not delete the test."
received = []
host_started = time.monotonic()


def host_event(event, **fields):
    print(json.dumps({'protocol': 1, 'event': event, 'scenario': scenario,
                      'strategy': strategy, 'elapsed_ms': (time.monotonic()-host_started)*1000,
                      **fields}), flush=True)


def draft_visible(output, text):
    visible = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', output).decode('utf-8', errors='replace')
    # The renderer can position words with cursor moves instead of literal spaces.
    return '❯' in visible and ''.join(visible.rsplit('❯', 1)[-1].split()).startswith(''.join(text.split()))


def provider_has_text(body, text):
    for message in body.get('messages', []):
        if message.get('role') != 'user':
            continue
        content = message.get('content')
        if content == text or isinstance(content, list) and any(
                isinstance(block, dict) and block.get('type') == 'text' and block.get('text') == text
                for block in content):
            return True
    return False


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
        print(json.dumps({'error':'host_prerequisite_missing'}), flush=True)
        raise SystemExit("claude and CRAIG_MODEL_DIR are required")
    with open(binary, 'rb') as executable:
        if executable.read(2) == b'#!':
            print(json.dumps({'error':'host_wrapper_requires_native_binary'}), flush=True)
            raise SystemExit('set CRAIG_CLAUDE_BINARY to the installed native executable; wrappers can override the isolated profile')
        executable.seek(0)
        binary_hash = hashlib.sha256()
        for block in iter(lambda: executable.read(1024 * 1024), b''):  # 1 MiB
            binary_hash.update(block)
    with tempfile.TemporaryDirectory(prefix="craig-host-") as directory:
        home = Path(directory).resolve()
        profile = home / ".claude"
        profile.mkdir()
        workspace = home / "workspace"
        workspace.mkdir()
        plugin = root
        if scenario in ('processing_timeout', 'worker_exit'):
            plugin = home / 'plugin'
            plugin.mkdir()
            for name in ('.claude-plugin', 'hooks', 'src', 'bridge'):
                shutil.copytree(root / name, plugin / name)
            for name in ('native', 'target', 'node_modules'):
                (plugin / name).symlink_to(root / name, target_is_directory=True)
            shutil.copyfile(root / 'tests/fixtures/host-failure.mjs', plugin / 'bridge/cli.mjs')
        config = {"hasCompletedOnboarding": True, "theme": "dark", "numStartups": 1, "projects": {str(workspace): {"hasTrustDialogAccepted": False, "hasCompletedProjectOnboarding": True}}, "customApiKeyResponses": {"approved": ["synthetic-local-key"], "rejected": []}}
        (home / ".claude.json").write_text(json.dumps(config))
        (profile / ".claude.json").write_text(json.dumps(config))
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Provider)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        env = {"PATH": os.environ["PATH"], "HOME": str(home), "CLAUDE_CONFIG_DIR": str(profile), "TERM": "xterm-256color", "ANTHROPIC_API_KEY": "synthetic-local-key", "ANTHROPIC_BASE_URL": f"http://127.0.0.1:{server.server_port}", "CRAIG_MODEL_DIR": str(Path(model).resolve()), "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1", "DISABLE_AUTOUPDATER": "1", "DISABLE_TELEMETRY": "1", "DISABLE_ERROR_REPORTING": "1", "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1", "ENABLE_CLAUDEAI_MCP_SERVERS": "false", "OTEL_SDK_DISABLED": "true"}
        env["CRAIG_DETECTOR"] = os.environ.get("CRAIG_DETECTOR", "bert")
        env["CRAIG_REPLACEMENT_STRATEGY"] = strategy
        env['CRAIG_HOST_SCENARIO'] = scenario
        if scenario == 'missing_model':
            env['CRAIG_MODEL_DIR'] = str(home / 'missing-model')
        if "CRAIG_REPLACEMENT" in os.environ:
            env["CRAIG_REPLACEMENT"] = os.environ["CRAIG_REPLACEMENT"]
        for key in ('SYSTEMROOT','WINDIR','COMSPEC','TEMP','TMP'):
            if key in os.environ: env[key]=os.environ[key]
        env['USERPROFILE']=str(home)
        version = subprocess.run([binary, "--version"], cwd=workspace, env=env, text=True, capture_output=True, check=True, timeout=10)
        if not version.stdout.startswith("2.1.290 "):
            server.shutdown()
            print(json.dumps({'error':'host_version_unsupported'}), flush=True)
            raise SystemExit("installed UI fixture requires claude 2.1.290")
        terminal = Terminal([binary, "--plugin-dir", str(plugin), "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}', "--setting-sources", "", "--tools", "", "--model", "claude-sonnet-4-5", "--system-prompt", "Respond with synthetic response."], workspace, env)
        output = b""
        try:
            stage='ready'
            host_event('ready')
            deadline=time.monotonic()+45
            submitted_at=None
            while time.monotonic()<deadline:
                output=(output+terminal.read())[-65536:]  # 64 KiB, held only in memory
                if not terminal.alive():
                    raise RuntimeError(f'{stage}: installed_host_exited')
                if any(marker in output for marker in [b'oauth/authorize',b'Paste code',b'Login successful']):
                    raise RuntimeError(f'{stage}: unexpected_authentication')
                visible=re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]',b'',output)
                if stage in ('failure_submission', 'recovery_ready') and received:
                    raise RuntimeError(f'{stage}: provider_request_observed')
                if stage in ('submission', 'recovery_submission') and any(
                        source in json.dumps(body) for body in received
                        for source in (original, 'You idiot, do not delete the test.')):
                    raise RuntimeError(f'{stage}: original_reached_provider')
                if stage=='ready' and b'trust' in visible and b'No,' in visible:
                    # The pinned host redraws its initial selection during startup.
                    # Drain that output before navigating the synthetic workspace dialog.
                    trust_ready_at=time.monotonic()+1
                    stage='trust_ready';output=b'';deadline=time.monotonic()+10
                elif stage=='trust_ready' and time.monotonic()>=trust_ready_at:
                    terminal.write(b'\x1b[B')
                    host_event('trust_selection')
                    stage='trust_selection';output=b'';deadline=time.monotonic()+10
                elif stage=='trust_selection' and re.search('❯\\s*Yes,\\s*I\\s*trust\\s*this\\s*folder'.encode(),visible):
                    # Wait for the selection redraw before sending Enter; a combined
                    # arrow/Enter write can be consumed as one terminal input event.
                    terminal.write(b'\r')
                    host_event('trusted')
                    stage='trusted';output=b'';deadline=time.monotonic()+45
                elif stage in ('ready','trusted') and '❯'.encode() in visible and b'No,' not in visible:
                    terminal.write(b'/craig')
                    stage='command';output=b'';deadline=time.monotonic()+10
                elif stage=='command' and b'/craig' in visible:
                    terminal.write(b'\r')
                    host_event('pane')
                    stage='pane';output=b'';deadline=time.monotonic()+30
                elif stage=='pane' and b'personal metrics' in visible:
                    if scenario != 'success':
                        terminal.write(b'\x1b')
                        time.sleep(0.2)
                        terminal.write(original.encode()+b'\r')
                        stage='failure_submission';output=b'';deadline=time.monotonic()+60
                        host_event('failure_submission')
                        continue
                    terminal.write(b'm')
                    host_event('metrics')
                    stage='metrics';output=b'';deadline=time.monotonic()+30
                elif stage=='metrics' and b'enable live metrics and sampled local judge' in visible:
                    terminal.write(b'l')
                    host_event('consent')
                    stage='consent';output=b'';deadline=time.monotonic()+30
                elif stage=='consent' and b'disable live metrics' in visible:
                    state=bridge_request(['node',str(root/'bridge/review.mjs')],{'command':'status'},env,stage)
                    if state['consent']['live'] is not True or state['consent']['history'] is not False:
                        raise RuntimeError('consent: ui_worker_mismatch')
                    terminal.write(b'\x1b')
                    time.sleep(0.2)
                    terminal.write(original.encode()+b'\r')
                    submitted_at=time.monotonic()
                    host_event('submission')
                    stage='submission';output=b'';deadline=time.monotonic()+60
                elif stage=='submission' and any(provider_has_text(body, replacement) for body in received):
                    if any(original in json.dumps(body) for body in received):
                        raise RuntimeError('submission: original_reached_provider')
                    submission_ms=(time.monotonic()-submitted_at)*1000
                    host_event('provider_received')
                    host_event('metrics_recorded')
                    stage='metrics_recorded';deadline=time.monotonic()+15
                elif stage=='failure_submission' and b'local replacement failed' in visible:
                    if received:
                        raise RuntimeError('failure_submission: provider_request_observed')
                    host_event('failure_detected', no_request_on_failure=True)
                    # Recovery without retyping proves the host kept the failed draft.
                    if scenario in ('processing_timeout', 'worker_exit'):
                        shutil.copyfile(root / 'bridge/cli.mjs', plugin / 'bridge/cli.mjs')
                    elif scenario == 'missing_model':
                        (home / 'missing-model').symlink_to(Path(model).resolve(), target_is_directory=True)
                    stage='recovery_ready';output=b'';deadline=time.monotonic()+10
                    # A fresh redraw distinguishes the retained draft from its submitted echo.
                    terminal.write(b'\x0c')
                elif stage=='recovery_ready' and draft_visible(output, original):
                    if received:
                        raise RuntimeError('recovery_ready: provider_request_observed')
                    host_event('draft_ready')
                    if scenario == 'protected':
                        terminal.write(b'\x01\x0b')
                        terminal.write(b'You idiot, do not delete the test.')
                    terminal.write(b'\r')
                    stage='recovery_submission';output=b'';deadline=time.monotonic()+60
                    submitted_at=time.monotonic()
                    host_event('recovery_submission')
                elif stage=='recovery_submission' and any(provider_has_text(body, replacement) for body in received):
                    if any(original in json.dumps(body) for body in received):
                        raise RuntimeError('recovery_submission: original_reached_provider')
                    host_event('provider_received')
                    host_event('completed', **{'host_version':version.stdout.strip(),
                        'host_sha256':binary_hash.hexdigest(),'no_request_on_failure':True,
                        'recovery':'edited_draft' if scenario=='protected' else 'retained_draft_retry',
                        'injection':'bridge_response' if scenario=='processing_timeout' else 'bridge_process_exit' if scenario=='worker_exit' else 'none',
                        'submission_to_provider_ms':(time.monotonic()-submitted_at)*1000,'samples':1})
                    return
                elif stage=='metrics_recorded':
                    metrics=bridge_request(['node',str(root/'bridge/review.mjs')],{'command':'status'},env,stage)
                    if metrics['consent']['live'] is not True or metrics['consent']['history'] is not False:
                        raise RuntimeError('metrics_recorded: unexpected_consent')
                    if metrics['summary']['live']['observations']!=1:
                        continue
                    assert metrics['summary']['live']['removed_characters']==5
                    assert original.encode() not in (profile/'craig/metrics.sqlite3').read_bytes()
                    assert original not in json.dumps(metrics)
                    host_event('completed', host_version=version.stdout.strip(),host_sha256=binary_hash.hexdigest(),submission_to_provider_ms=submission_ms,samples=1)
                    print('installed mod: UI opt-in, native assessment, text-free metrics and sanitized provider payload passed')
                    return
            raise RuntimeError(f'{stage}: ' + ('draft_not_observed' if stage=='recovery_ready' else 'installed_host_timeout'))
        except Exception as error:
            reason = str(error).split(': ')[-1]
            allowed = {'installed_host_timeout', 'installed_host_exited', 'unexpected_authentication',
                       'provider_request_observed', 'original_reached_provider', 'draft_not_observed',
                       'unexpected_consent', 'ui_worker_mismatch'}
            phase = {'recovery_ready':'failure_detected', 'command':'trusted', 'trust_ready':'ready'}.get(stage, stage)
            host_event('failed', phase=phase, reason=reason if reason in allowed else 'host_execution_failed',
                       draft_text_visible=original.encode() in visible, prompt_marker_visible='❯'.encode() in visible,
                       draft_words_visible=all(word.encode() in visible for word in original.split()))
            raise RuntimeError('installed_host_failed') from None
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
