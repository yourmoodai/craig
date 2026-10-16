"""Qualification must stop when further inference cannot make the gate pass."""

import io
import unittest
import ast
import shutil
import subprocess
from contextlib import redirect_stdout
from unittest.mock import patch

import evaluate_model
from runtime_facts import runtime_facts


class TestModelQualification(unittest.TestCase):
    @unittest.skipUnless(shutil.which('node'), 'node required')
    def test_node_qualification_labels_match_actual_execution(self):
        for name, _, change, expected in evaluate_model.CASES[:2]:
            with self.subTest(case=name):
                for prefix in ['-', '+']:
                    statement=next(line[1:] for line in change.splitlines() if line.startswith(prefix))
                    call=ast.parse(statement).body[0].value
                    argv=ast.literal_eval(call.args[0])
                    result=subprocess.run(argv,capture_output=True,text=True,timeout=10)
                    valid=prefix=='-' or expected=='APPROVE'
                    self.assertEqual(result.returncode==0,valid,result.stderr)
                    if valid:
                        self.assertEqual(result.stdout.strip(),'42')
                    else:
                        self.assertIn('SyntaxError',result.stderr)

    @unittest.skipUnless(shutil.which('node'), 'node required')
    def test_trusted_runtime_probes_distinguish_binding_scopes(self):
        import json
        evidence=json.loads(runtime_facts().split(': ',1)[1])
        allowed, duplicate=evidence['observations']
        self.assertEqual(allowed['exit_code'],0)
        self.assertTrue(allowed['printed_42'])
        self.assertNotEqual(duplicate['exit_code'],0)
        self.assertTrue(duplicate['duplicate_declaration_error'])

    def test_failed_case_stops_qualification(self):
        with (
            patch("sys.argv", ["evaluate_model"]),
            patch.object(
                evaluate_model,
                "run_model_reviewer",
                return_value=("COMMENT", [], "review incomplete: runner timed out."),
            ) as reviewer,
            redirect_stdout(io.StringIO()) as output,
        ):
            self.assertEqual(evaluate_model.main(), 1)
        self.assertEqual(reviewer.call_count, evaluate_model.REVIEW_WORKERS)
        self.assertIn(f"starting case 1/{len(evaluate_model.CASES)}", output.getvalue())
        self.assertIn('"duration_seconds"', output.getvalue())

    def test_all_cases_share_qualification_deadline(self):
        with (
            patch("sys.argv", ["evaluate_model"]),
            patch.object(
                evaluate_model,
                "run_model_reviewer",
                side_effect=lambda diff, *args, **kwargs: (
                    next(case[3] for case in evaluate_model.CASES if case[2] in diff),
                    [],
                    "assessed",
                ),
            ) as reviewer,
            redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(evaluate_model.main(), 0)
        deadlines = [call.kwargs["deadline"] for call in reviewer.call_args_list]
        self.assertEqual(len(deadlines), len(evaluate_model.CASES))
        self.assertEqual(len(set(deadlines)), 1)
