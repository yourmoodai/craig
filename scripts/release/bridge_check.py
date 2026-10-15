"""Bounded bridge checks that never include response text in diagnostics."""
import json
import subprocess


def request(argv, command, env, stage, timeout=15):
    try:
        result = subprocess.run(argv, input=json.dumps(command), text=True,
                                capture_output=True, env=env, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f'{stage}: bridge_timeout') from None
    if result.returncode:
        raise RuntimeError(f'{stage}: bridge_exit_{result.returncode}')
    if not result.stdout.strip():
        raise RuntimeError(f'{stage}: bridge_empty_output')
    try:
        response = json.loads(result.stdout)
    except ValueError:
        raise RuntimeError(f'{stage}: bridge_invalid_json') from None
    if not isinstance(response, dict) or response.get('version') != 1 or response.get('status') != 'ok' or not isinstance(response.get('value'), dict):
        raise RuntimeError(f'{stage}: bridge_invalid_response')
    return response['value']
