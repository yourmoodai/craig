"""Fixed reviewer-owned probes; never execute code from a pull request."""
from functools import cache
import json
import re
import subprocess


@cache
def runtime_facts():
    try:
        version = subprocess.run(['node', '--version'], capture_output=True, text=True, timeout=10, check=True).stdout.strip()
        if not re.fullmatch(r'v\d+\.\d+\.\d+', version):
            return 'Node runtime probes unavailable.'
        facts = []
        for script in (
            'const require = () => 42; console.log(require())',
            'const require = () => 42; const require = () => 43; console.log(require())',
        ):
            result = subprocess.run(['node', '--input-type=commonjs', '-e', script], capture_output=True, text=True, timeout=10)
            facts.append({'script':script, 'exit_code':result.returncode,
                          'printed_42':result.stdout.strip()=='42',
                          'duplicate_declaration_error':"Identifier 'require' has already been declared" in result.stderr})
        return 'Trusted execution evidence from fixed reviewer-owned probes (not PR code): ' + json.dumps({'node':version, 'mode':'--input-type=commonjs -e', 'observations':facts})
    except (OSError, subprocess.SubprocessError):
        return 'Node runtime probes unavailable.'
