"""Offline tests. Every temporary directory is deliberately retained."""
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('runner', Path(__file__).with_name('runner.py'))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class RunnerTests(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).parent / 'test-artifacts'
        root.mkdir(exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='runner-', dir=root))

    def test_interruption_consumes_slot_and_is_marked_uncertain(self):
        path = self.root / 'ledger.jsonl'
        with runner.Ledger(path) as ledger:
            first = ledger.reserve('model', 'digest', 'trial', 0, 'prompt', limit=3)
        with runner.Ledger(path) as ledger:
            self.assertEqual(ledger.events[-1]['status'], 'uncertain')
            ledger.reserve('model', 'digest', 'trial', 1, 'prompt', limit=3)
        with runner.Ledger(path) as ledger:
            ledger.reserve('model', 'digest', 'trial', 2, 'prompt', limit=3)
            with self.assertRaises(RuntimeError):
                ledger.reserve('model', 'digest', 'trial', 3, 'prompt', limit=3)
        self.assertTrue(first)

    def test_aggregate_limit_counts_distinct_trials(self):
        with runner.Ledger(self.root / 'ledger.jsonl') as ledger:
            for n in range(30):
                ledger.reserve('m', 'd', f'trial-{n}', 0, 'p')
            with self.assertRaises(RuntimeError):
                ledger.reserve('m', 'd', 'another', 0, 'p')

    def test_aliases_cannot_reset_digest_budget(self):
        with runner.Ledger(self.root / 'ledger.jsonl') as ledger:
            for n in range(30):
                ledger.reserve(f'alias-{n}', 'same-digest', f'trial-{n}', 0, 'p')
            with self.assertRaises(RuntimeError):
                ledger.reserve('new-alias', 'same-digest', 'another', 0, 'p')

    def test_reservation_is_durable_before_network(self):
        protocol_path = self.root / 'protocol.json'
        protocol_path.write_text('{}')
        frozen = {'snapshot': runner.snapshot([protocol_path])}
        protocol = {'ledger_file': 'calls.jsonl'}
        attempt_dir = self.root / 'attempt'
        attempt_dir.mkdir()
        def transport(*args, **kwargs):
            events = [json.loads(line) for line in (self.root / 'calls.jsonl').read_text().splitlines()]
            self.assertEqual(events[-1]['event'], 'reserve')
            raise OSError('interrupted transport')
        with patch.object(runner, 'verify_model_digest'), patch.object(runner.urllib.request, 'urlopen', side_effect=transport):
            with self.assertRaises(OSError):
                runner.infer(protocol_path, protocol, 'm', 'd', 't', 0, b'{}', attempt_dir, frozen)
        events = [json.loads(line) for line in (self.root / 'calls.jsonl').read_text().splitlines()]
        self.assertEqual(events[-1]['status'], 'uncertain')
        self.assertEqual(sum(e['event'] == 'reserve' for e in events), 1)

    def test_changed_inputs_prevent_network(self):
        protocol_path = self.root / 'protocol.json'
        protocol_path.write_text('{}')
        frozen = {'snapshot': runner.snapshot([protocol_path])}
        protocol_path.write_text('changed')
        attempt_dir = self.root / 'attempt'
        attempt_dir.mkdir()
        with patch.object(runner.urllib.request, 'urlopen') as network:
            with self.assertRaises(RuntimeError):
                runner.infer(protocol_path, {'ledger_file': 'calls.jsonl'}, 'm', 'd', 't', 0, b'{}', attempt_dir, frozen)
        network.assert_not_called()

    def test_snapshot_detects_added_and_changed_files(self):
        source = self.root / 'source'
        source.mkdir()
        (source / 'lib.rs').write_text('original')
        frozen = runner.snapshot([source])
        runner.verify_snapshot(frozen)
        (source / 'extra.rs').write_text('new')
        with self.assertRaises(RuntimeError):
            runner.verify_snapshot(frozen)

    def test_clearance_must_bind_exact_snapshot_and_models(self):
        frozen = {'protocol_sha256': 'p', 'model_digests': {'m': 'd'}}
        path = self.root / 'frozen.json'
        path.write_text(json.dumps(frozen))
        clearance = {'frozen_sha256': runner.sha256_file(path), 'protocol_sha256': 'p',
                     'model_digests': {'m': 'd'}, 'human_approved': True,
                     'approved_by': 'Gianni',
                     'host_reservation': {'id': 'reservation', 'host': runner.socket.gethostname(), 'exclusive': True}}
        runner.validate_clearance(clearance, path, frozen)
        clearance['model_digests']['m'] = 'wrong'
        with self.assertRaises(RuntimeError):
            runner.validate_clearance(clearance, path, frozen)

    def test_candidate_extraction_requires_single_rust_answer(self):
        self.assertEqual(runner.extract_code('```rust\nfn a() {}\n```'), 'fn a() {}\n')
        with self.assertRaises(ValueError):
            runner.extract_code('```rust\nfn a() {}\n```\n```rust\nfn b() {}\n```')

    def test_context_byte_limits_and_support_outside_cap(self):
        protocol_path = self.root / 'protocol.json'
        protocol = {'context_files': ['context.md'], 'reduced_context_files': ['context.md'], 'support_file': 'support.rs'}
        (self.root / 'task.md').write_text('task')
        (self.root / 'support.rs').write_text('SUPPORT VERBATIM' * 1000)
        case = {'task': 'task.md', 'track': 'reduction_repeat'}
        (self.root / 'context.md').write_text('a' * 7900)
        self.assertIn('SUPPORT VERBATIM' * 1000, runner.prompt_for(protocol_path, protocol, case))
        (self.root / 'context.md').write_text('é' * 4001)
        with self.assertRaises(ValueError):
            runner.prompt_for(protocol_path, protocol, case)
        case['track'] = 'fresh'
        (self.root / 'context.md').write_text('é' * 8001)
        with self.assertRaises(ValueError):
            runner.prompt_for(protocol_path, protocol, case)

    def test_result_dimensions_do_not_assume_diagnostics_or_structure(self):
        result = runner.classify_build(0, 'test assertions::behavior ... ok\n')
        self.assertEqual(result['compile'], 'passed')
        self.assertEqual(result['behavior'], 'passed')
        self.assertEqual(result['diagnostic'], 'not_measured')
        self.assertEqual(result['structural_review'], 'pending')
        fake = runner.classify_build(0, 'test candidate::behavior ... ok\ntest candidate::diagnostic ... ok\n')
        self.assertEqual(fake['behavior'], 'not_measured')
        self.assertEqual(fake['diagnostic'], 'not_measured')
        result = runner.classify_build(101, 'test assertions::behavior ... ok\ntest assertions::diagnostic ... FAILED\n')
        self.assertEqual(result['compile'], 'passed')
        self.assertEqual(result['diagnostic'], 'failed')

    def test_postbuild_evaluator_mutation_is_detected_and_logs_retained(self):
        protocol_path = self.root / 'protocol.json'
        inputs = self.root / 'inputs'
        inputs.mkdir()
        for filename in ('Cargo.toml', 'Cargo.lock', 'support.rs', 'assertions.rs'):
            (inputs / filename).write_text('frozen input')
        protocol = {'cargo_template_file': 'inputs/Cargo.toml', 'cargo_lock_file': 'inputs/Cargo.lock',
                    'support_file': 'inputs/support.rs', 'sdax_rs': '.'}
        attempt = self.root / 'build-attempt'
        attempt.mkdir()
        def mutate(*args, **kwargs):
            (attempt / 'src/lib.rs').write_text('mutated evaluator')
            return SimpleNamespace(stdout=b'test assertions::behavior ... ok\n', stderr=b'', returncode=0)
        with patch.object(runner.subprocess, 'run', side_effect=mutate):
            with self.assertRaises(RuntimeError):
                runner.build_attempt(protocol_path, protocol, {'assertions': 'inputs/assertions.rs'},
                                     attempt, 'candidate code', {'snapshot': runner.snapshot([inputs])})
        self.assertTrue((attempt / 'build.stdout').exists())
        self.assertTrue((attempt / 'build.json').exists())

    def test_budget_failure_records_zero_calls_without_inference(self):
        inputs = self.root / 'inputs'
        inputs.mkdir()
        protocol_path = inputs / 'protocol.json'
        protocol_path.write_text('{}')
        (inputs / 'task.md').write_text('task')
        clearance = self.root / 'clearance.json'
        clearance.write_text('{}')
        protocol = {'context_files': ['context.md'], 'support_file': 'support.rs', 'run_root': str(self.root / 'runs')}
        cases = [{'id': 't', 'track': 'fresh', 'task': 'task.md'}]
        for context_size, support_size in [(16001, 1), (1, 64001)]:
            (inputs / 'context.md').write_text('c' * context_size)
            (inputs / 'support.rs').write_text('s' * support_size)
            frozen = {'snapshot': runner.snapshot([inputs]), 'model_digests': {'m': 'd'},
                      'protocol_path': str(protocol_path)}
            frozen_path = self.root / f'frozen-{context_size}.json'
            frozen_path.write_text(json.dumps(frozen))
            with patch.object(runner, 'load_protocol', return_value=(protocol_path, protocol, cases)), \
                 patch.object(runner, 'validate_clearance'), patch.object(runner, 'infer') as inference:
                run_dir = runner.run(frozen_path, clearance)
            inference.assert_not_called()
            result = json.loads((run_dir / 'results.json').read_text())[0]
            self.assertEqual(result['status'], 'budget_failure')
            self.assertEqual(result['inference_calls'], 0)

    def test_zero_exit_without_frozen_assertions_is_not_pass(self):
        inputs = self.root / 'inputs'
        inputs.mkdir()
        for filename in ('Cargo.toml', 'Cargo.lock', 'support.rs'):
            (inputs / filename).write_text('input')
        (inputs / 'assertions.rs').write_text('#[test] fn behavior() {}\n#[test] fn diagnostic() {}')
        protocol = {'cargo_template_file': 'inputs/Cargo.toml', 'cargo_lock_file': 'inputs/Cargo.lock',
                    'support_file': 'inputs/support.rs', 'sdax_rs': '.'}
        attempt = self.root / 'attempt'
        attempt.mkdir()
        result = SimpleNamespace(stdout=b'', stderr=b'', returncode=0)
        with patch.object(runner.subprocess, 'run', return_value=result):
            passed, diagnostics = runner.build_attempt(self.root / 'protocol.json', protocol,
                 {'assertions': 'inputs/assertions.rs'}, attempt, 'candidate', {'snapshot': runner.snapshot([inputs])})
        self.assertFalse(passed)
        self.assertIn('Frozen assertions did not all pass', diagnostics)

    def test_clearance_requires_matching_complete_control_run(self):
        frozen = {'protocol_sha256': 'p', 'model_digests': {'m': 'd'},
                  'cases': [{'id': f'case-{n}'} for n in range(10)]}
        frozen_path = self.root / 'frozen.json'
        frozen_path.write_text(json.dumps(frozen))
        control = self.root / 'control'
        control.mkdir()
        (control / 'frozen.json').write_text(json.dumps(frozen))
        clearance = {'frozen_sha256': runner.sha256_file(frozen_path), 'protocol_sha256': 'p',
                     'model_digests': {'m': 'd'}, 'human_approved': True, 'approved_by': 'Gianni',
                     'host_reservation': {'id': 'reservation', 'host': runner.socket.gethostname(), 'exclusive': True},
                     'control_run': str(control)}
        results = [{'model': 'control', 'case': c['id'], 'passed': True, 'compile': 'passed', 'behavior': 'passed'}
                   for c in frozen['cases']]
        (control / 'results.json').write_text(json.dumps(results[:-1]))
        with self.assertRaises(RuntimeError):
            runner.validate_clearance(clearance, frozen_path, frozen, {})
        (control / 'results.json').write_text(json.dumps(results))
        runner.validate_clearance(clearance, frozen_path, frozen, {})
        (control / 'frozen.json').write_text('{}')
        with self.assertRaises(RuntimeError):
            runner.validate_clearance(clearance, frozen_path, frozen, {})

    def test_resume_skips_success_and_preserves_failed_attempt_slots(self):
        attempt = self.root / 'm' / 't' / '0'
        attempt.mkdir(parents=True)
        event = {'event': 'reserve', 'model': 'm', 'trial': 't', 'attempt': 0}
        (attempt / 'result.json').write_text(json.dumps({'passed': True}))
        self.assertEqual(runner.resume_trial(self.root, 'm', 't', [event])[0], 3)
        (attempt / 'result.json').write_text(json.dumps({'passed': False, 'diagnostics': 'exact compiler diagnostic'}))
        (attempt / 'answer.txt').write_text('latest exact answer')
        first, answer, diagnostics, records = runner.resume_trial(self.root, 'm', 't', [event])
        self.assertEqual((first, answer, diagnostics), (1, 'latest exact answer', 'exact compiler diagnostic'))
        self.assertEqual(records[0]['structural_review'], 'pending')

    def test_resume_retains_zero_call_budget_failure(self):
        attempt = self.root / 'm' / 't' / '0'
        attempt.mkdir(parents=True)
        record = {'model': 'm', 'case': 't', 'attempt': 0, 'passed': False,
                  'status': 'budget_failure', 'inference_calls': 0}
        (attempt / 'result.json').write_text(json.dumps(record))
        first, _, _, records = runner.resume_trial(self.root, 'm', 't', [])
        self.assertEqual(first, 3)
        self.assertEqual(records[0]['status'], 'budget_failure')
        self.assertEqual(records[0]['inference_calls'], 0)

    def test_resume_persists_every_uncertain_charged_slot(self):
        attempt = self.root / 'm' / 't' / '0'
        attempt.mkdir(parents=True)
        events = [{'event': 'reserve', 'model': 'm', 'trial': 't', 'attempt': 0, 'call_id': 'c'},
                  {'event': 'terminal', 'call_id': 'c', 'status': 'uncertain', 'detail': 'connection interrupted'}]
        first, _, diagnostics, records = runner.resume_trial(self.root, 'm', 't', events)
        self.assertEqual(first, 1)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]['status'], 'uncertain')
        self.assertEqual(records[0]['original_event_status'], 'uncertain')
        self.assertEqual(records[0]['inference_calls'], 1)
        self.assertIn('connection interrupted', diagnostics)
        self.assertTrue((attempt / 'result.json').exists())

    def test_resume_corrupt_response_preserves_raw_evidence(self):
        attempt = self.root / 'm' / 't' / '0'
        attempt.mkdir(parents=True)
        raw = b'{broken response'
        (attempt / 'response.json').write_bytes(raw)
        (attempt / 'inference-error.json').write_text(json.dumps({'error': 'JSONDecodeError: exact failure'}))
        events = [{'event': 'reserve', 'model': 'm', 'trial': 't', 'attempt': 0, 'call_id': 'c'},
                  {'event': 'terminal', 'call_id': 'c', 'status': 'uncertain'}]
        first, answer, diagnostics, records = runner.resume_trial(self.root, 'm', 't', events)
        self.assertEqual((first, answer, diagnostics), (1, '', 'JSONDecodeError: exact failure'))
        self.assertEqual((attempt / 'response.json').read_bytes(), raw)
        self.assertEqual(records[0]['inference_calls'], 1)

    def test_incomplete_test_inventory_cannot_pass_dimension(self):
        result = runner.classify_build(124, 'test assertions::behavior_first ... ok\nRunner: cargo test timeout\n',
                                       {'behavior_first', 'behavior_second'})
        self.assertEqual(result['behavior'], 'incomplete')
        result = runner.classify_build(0, 'test assertions::behavior_first ... ok\ntest assertions::behavior_second ... ok\n',
                                       {'behavior_first', 'behavior_second'})
        self.assertEqual(result['behavior'], 'passed')

    def test_uncertain_resume_needs_explicit_bound_reconciliation(self):
        frozen_path = self.root / 'frozen.json'
        frozen_path.write_text('{}')
        events = [{'event': 'reserve', 'call_id': 'call-1'}, {'event': 'terminal', 'call_id': 'call-1', 'status': 'uncertain'}]
        with self.assertRaises(RuntimeError):
            runner.validate_reconciliation(None, frozen_path, events)
        record = {'frozen_sha256': runner.sha256_file(frozen_path), 'human_approved': True,
                  'approved_by': 'Gianni', 'consumed_call_ids': ['call-1']}
        runner.validate_reconciliation(record, frozen_path, events)
        record['consumed_call_ids'] = []
        with self.assertRaises(RuntimeError):
            runner.validate_reconciliation(record, frozen_path, events)

    def test_request_limits_and_fixed_options(self):
        data = runner.request_body('model', 'task', 8000)
        parsed = json.loads(data)
        self.assertIs(parsed['think'], False)
        self.assertEqual(parsed['options'], {'num_predict': 4096, 'temperature': 0, 'seed': 42, 'num_ctx': 32768})
        with self.assertRaises(ValueError):
            runner.request_body('model', 'x' * 64001, 8000)


if __name__ == '__main__':
    unittest.main()
