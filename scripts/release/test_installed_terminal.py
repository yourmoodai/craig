import os
from pathlib import Path
import subprocess
import sys
import unittest
import importlib.util

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
spec = importlib.util.spec_from_file_location('installed', Path(__file__).resolve().parents[1] / 'test-installed-mod.py')
installed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installed)


class InstalledTerminalTests(unittest.TestCase):
    def test_draft_readiness_requires_the_latest_prompt_marker(self):
        self.assertTrue(installed.draft_visible('❯ You\u00a0idiot, keep it.'.encode(), 'You idiot, keep it.'))
        self.assertTrue(installed.draft_visible('❯ You \x1b[31midiot,\x1b[0m keep it.'.encode(), 'You idiot, keep it.'))
        self.assertTrue(installed.draft_visible('❯You\x1b[1Cidiot,\x1b[1Ckeep\x1b[1Cit.'.encode(), 'You idiot, keep it.'))
        self.assertFalse(installed.draft_visible('❯ You idiot, keep it.\n❯ '.encode(), 'You idiot, keep it.'))
        self.assertFalse(installed.draft_visible(b'You idiot, keep it.', 'You idiot, keep it.'))
        self.assertFalse(installed.draft_visible('❯ newer draft'.encode(), 'You idiot, keep it.'))

    def test_provider_requires_exact_user_text_not_a_substring(self):
        self.assertTrue(installed.provider_has_text({'messages':[{'role':'user','content':'expected'}]}, 'expected'))
        self.assertTrue(installed.provider_has_text({'messages':[{'role':'user','content':[{'type':'text','text':'expected'}]}]}, 'expected'))
        self.assertFalse(installed.provider_has_text({'messages':[{'role':'user','content':'unexpected'}]}, 'expected'))
        self.assertFalse(installed.provider_has_text({'messages':[{'role':'assistant','content':'expected'}]}, 'expected'))

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
