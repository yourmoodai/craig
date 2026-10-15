import os
from pathlib import Path
import subprocess
import sys
import unittest


class InstalledTerminalTests(unittest.TestCase):
    @unittest.skipUnless(os.name == 'posix', 'requires a Unix PTY')
    def test_close_unblocks_an_active_reader(self):
        root = Path(__file__).resolve().parents[2]
        # Isolate the close call so a regression fails within a bounded deadline.
        code = '''
import importlib.util
import os
import sys
import time
sys.path.insert(0, 'scripts')
spec = importlib.util.spec_from_file_location('installed', 'scripts/test-installed-mod.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
terminal = module.Terminal([sys.executable, '-c', 'import time; print("ready", flush=True); time.sleep(30)'], os.getcwd(), dict(os.environ))
deadline = time.monotonic() + 5
while b'ready' not in terminal.read():
    assert time.monotonic() < deadline
terminal.close()
assert not terminal.alive()
'''
        subprocess.run([sys.executable, '-c', code], cwd=root, check=True, timeout=10)
