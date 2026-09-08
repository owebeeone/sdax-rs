#!/usr/bin/env python3
"""Retained, frozen-fixture Ollama evaluation. No inference occurs before clearance."""
import argparse
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import time
import urllib.request
import uuid


def sha256_file(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def snapshot(paths):
    roots = sorted({str(Path(p).resolve()) for p in paths})
    files = {}
    for root in roots:
        path = Path(root)
        if not path.exists():
            raise RuntimeError(f'Missing immutable input: {path}')
        entries = sorted(path.rglob('*')) if path.is_dir() else [path]
        for item in entries:
            if item.is_symlink():
                raise RuntimeError(f'Symlink in immutable inputs: {item}')
            if item.is_file():
                files[str(item)] = sha256_file(item)
    return {'roots': roots, 'files': files}


def verify_snapshot(expected):
    if snapshot(expected['roots']) != expected:
        raise RuntimeError('Immutable evaluator/source/context/protocol/lockfile snapshot changed')


class Ledger:
    """One shared append-only ledger; exclusive lock spans each network call.

    The fsynced reservation is the charge. Process death cannot refund it.
    Pending reservations found after acquiring the lock become uncertain.
    """
    def __init__(self, path):
        self.path = Path(path)

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.stream = self.path.open('a+')
        fcntl.flock(self.stream, fcntl.LOCK_EX)
        self.stream.seek(0)
        try:
            self.events = [json.loads(line) for line in self.stream if line.strip()]
        except (ValueError, OSError):
            self.stream.close()
            raise RuntimeError('Corrupt accounting ledger: refusing inference')
        ended = {e['call_id'] for e in self.events if e['event'] == 'terminal'}
        for event in list(self.events):
            if event['event'] == 'reserve' and event['call_id'] not in ended:
                self.finish(event['call_id'], 'uncertain', 'Process stopped before durable terminal event')
        return self

    def append(self, event):
        event['time_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        self.stream.write(json.dumps(event, sort_keys=True) + '\n')
        self.stream.flush()
        os.fsync(self.stream.fileno())
        descriptor = os.open(self.path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        self.events.append(event)

    def reserve(self, model, digest, trial, attempt, prompt_hash, limit=30):
        charged = [e for e in self.events if e['event'] == 'reserve' and
                   (e['model'] == model or e['digest'].removeprefix('sha256:') == digest.removeprefix('sha256:'))]
        if len(charged) >= min(limit, 30):
            raise RuntimeError('Aggregate per-model 30-call limit exhausted')
        previous = [e for e in charged if e['trial'] == trial]
        if len(previous) >= 3 or attempt not in (0, 1, 2) or any(e['attempt'] == attempt for e in previous):
            raise RuntimeError('Trial initial-plus-two-repairs budget exhausted or slot already charged')
        call_id = uuid.uuid4().hex
        self.append({'event': 'reserve', 'call_id': call_id, 'model': model,
                     'digest': digest, 'trial': trial, 'attempt': attempt,
                     'prompt_sha256': prompt_hash, 'status': 'reserved'})
        return call_id

    def finish(self, call_id, status, detail=''):
        self.append({'event': 'terminal', 'call_id': call_id, 'status': status, 'detail': detail})

    def __exit__(self, *_):
        fcntl.flock(self.stream, fcntl.LOCK_UN)
        self.stream.close()


def validate_clearance(clearance, frozen_path, frozen, protocol=None):
    required = {'frozen_sha256': sha256_file(frozen_path),
                'protocol_sha256': frozen['protocol_sha256'],
                'model_digests': frozen['model_digests'], 'human_approved': True}
    if any(clearance.get(k) != v for k, v in required.items()):
        raise RuntimeError('Clearance does not bind this exact frozen candidate/protocol/models')
    reservation = clearance.get('host_reservation', {})
    if not clearance.get('approved_by') or not reservation.get('id') or reservation.get('exclusive') is not True:
        raise RuntimeError('Explicit human approval and exclusive host reservation required')
    if reservation.get('host') != socket.gethostname():
        raise RuntimeError('Host reservation does not cover this host')
    if protocol is not None:
        try:
            control = Path(clearance['control_run']).resolve()
            same = json.loads((control / 'frozen.json').read_text()) == frozen
            results = json.loads((control / 'results.json').read_text())
            expected = {c['id'] for c in frozen['cases']}
            valid = len(results) == len(expected) == 10 and {r['case'] for r in results} == expected
            valid = valid and all(r['model'] == 'control' and r['passed'] is True and
                     r['compile'] == r['behavior'] == 'passed' for r in results)
            if not same or not valid:
                raise ValueError('mismatched candidate or incomplete controls')
        except (KeyError, OSError, ValueError, TypeError) as error:
            raise RuntimeError('Clearance requires matching frozen control run with all ten cases passed') from error


def extract_code(answer):
    blocks = re.findall(r'```(?:rust|rs)?\s*\n(.*?)```', answer, re.DOTALL)
    if len(blocks) != 1:
        raise ValueError('Return exactly one fenced Rust code block containing candidate.rs')
    return blocks[0].rstrip() + '\n'


def request_body(model, prompt, context_cap):
    if context_cap not in (8000, 16000):
        raise ValueError('Context cap must be 16000 or 8000')
    body = json.dumps({'model': model, 'messages': [{'role': 'user', 'content': prompt}],
                       'stream': False, 'think': False,
                       'options': {'num_predict': 4096, 'temperature': 0, 'seed': 42,
                                   'num_ctx': 32768}}, ensure_ascii=False).encode()
    if len(body) > 64000:
        raise ValueError('Serialized request exceeds 64000 bytes')
    return body


def resolve(protocol_path, value):
    path = Path(value)
    return path if path.is_absolute() else protocol_path.parent / path


def load_protocol(path):
    path = Path(path).resolve()
    protocol = json.loads(path.read_text())
    cases = json.loads(resolve(path, protocol['cases_file']).read_text())
    if isinstance(cases, dict):
        cases = cases['cases']
    counts = {track: sum(c['track'] == track for c in cases) for track in ('reconstructed_exposed', 'fresh', 'reduction_repeat')}
    if counts != {'reconstructed_exposed': 2, 'fresh': 6, 'reduction_repeat': 2} or len({c['id'] for c in cases}) != 10:
        raise RuntimeError('Protocol requires 2 reconstructed exposed, 6 fresh, 2 reduced trials')
    full = {c['id'] for c in cases if c['track'] != 'reduction_repeat'}
    if any(c.get('repeat_of') not in full for c in cases if c['track'] == 'reduction_repeat'):
        raise RuntimeError('Reduced trials must preselect a full-context repeat')
    return path, protocol, cases


def freeze(protocol_path, destination):
    path, protocol, cases = load_protocol(protocol_path)
    models = protocol['models']
    if not models or any(not re.fullmatch(r'(?:sha256:)?[0-9a-f]{64}', m['digest']) for m in models.values()):
        raise RuntimeError('Every model requires a recorded exact SHA-256 digest')
    inputs = [path, Path(__file__).resolve(), resolve(path, protocol['cases_file'])]
    for key in ('support_file', 'cargo_template_file', 'cargo_lock_file'):
        inputs.append(resolve(path, protocol[key]))
    for key in ('context_files', 'reduced_context_files', 'snapshot_paths'):
        inputs.extend(resolve(path, p) for p in protocol.get(key, []))
    for case in cases:
        inputs.extend(resolve(path, case[k]) for k in ('task', 'assertions', 'control'))
    frozen = {'protocol_path': str(path), 'protocol_sha256': sha256_file(path),
              'model_digests': {name: model['digest'] for name, model in models.items()},
              'snapshot': snapshot(inputs), 'cases': cases}
    save(destination, frozen)
    return frozen


def prompt_for(path, protocol, case, answer=None, diagnostics=None):
    keys = 'reduced_context_files' if case['track'] == 'reduction_repeat' else 'context_files'
    context = '\n\n'.join(f'FILE: {p}\n{resolve(path, p).read_text()}' for p in protocol.get(keys, []))
    cap = 8000 if case['track'] == 'reduction_repeat' else 16000
    if len(context.encode('utf-8')) > cap:
        raise ValueError(f'Public reference context exceeds {cap} UTF-8 bytes')
    task = resolve(path, case['task']).read_text()
    prompt = task + '\n\nFROZEN CONTEXT:\n' + context
    prompt += '\n\nSUPPLIED FIXTURE API (support.rs, verbatim):\n' + resolve(path, protocol['support_file']).read_text()
    prompt += '\n\nReturn exactly one fenced Rust code block containing candidate.rs.'
    if answer is not None:
        prompt += '\n\nLATEST ANSWER (verbatim):\n' + answer
        prompt += '\n\nEXACT DIAGNOSTICS:\n' + diagnostics
        prompt += '\n\nRepair only the problems shown in these diagnostics.'
    return prompt


def classify_build(returncode, diagnostics, expected_tests=None):
    tests = re.findall(r'^test assertions::([^\s]+) \.\.\. (ok|FAILED|ignored)', diagnostics, re.MULTILINE)
    result = {'compile': 'passed' if returncode == 0 or tests else
              ('failed' if 'could not compile' in diagnostics else 'unknown'),
              'structural_review': 'pending'}
    inventory = set(expected_tests) if expected_tests is not None else {name for name, _ in tests}
    for dimension in ('behavior', 'diagnostic'):
        expected = {name for name in inventory if dimension in name.split('::')[-1]}
        observed = {name: status for name, status in tests if name in expected}
        result[dimension] = ('not_measured' if not expected else 'failed' if 'FAILED' in observed.values()
                             else 'passed' if all(observed.get(name) == 'ok' for name in expected)
                             else 'incomplete')
    return result


def build_attempt(path, protocol, case, attempt_dir, code, frozen):
    verify_snapshot(frozen['snapshot'])
    source = attempt_dir / 'src'
    source.mkdir()
    (source / 'candidate.rs').write_text(code)
    template = resolve(path, protocol['cargo_template_file']).read_text()
    template = template.replace('{{SDAX_RS}}', str(resolve(path, protocol['sdax_rs']).resolve()))
    (attempt_dir / 'Cargo.toml').write_text(template)
    (attempt_dir / 'Cargo.lock').write_bytes(resolve(path, protocol['cargo_lock_file']).read_bytes())
    attribute = lambda p: '#[path = ' + json.dumps(str(p.resolve())) + ']\n'
    evaluator = attribute(resolve(path, protocol['support_file'])) + 'mod support;\nmod candidate;\n'
    evaluator += '#[cfg(test)]\n' + attribute(resolve(path, case['assertions'])) + 'mod assertions;\n'
    (source / 'lib.rs').write_text(evaluator)
    immutable = snapshot([attempt_dir / 'Cargo.toml', attempt_dir / 'Cargo.lock', source / 'lib.rs'])
    save(attempt_dir / 'evaluator-snapshot.json', immutable)
    verify_snapshot(frozen['snapshot'])
    verify_snapshot(immutable)
    start = time.monotonic()
    command = ['cargo', 'test', '--locked', '--offline', '--manifest-path', str(attempt_dir / 'Cargo.toml')]
    env = dict(os.environ, CARGO_TARGET_DIR=str(attempt_dir / 'target'))
    interrupted = None
    try:
        result = subprocess.run(command, capture_output=True, timeout=protocol.get('build_timeout_seconds', 300), env=env)
        stdout, stderr, returncode = result.stdout, result.stderr, result.returncode
    except subprocess.TimeoutExpired as error:
        stdout, stderr, returncode = error.stdout or b'', error.stderr or b'', 124
        stderr += b'\nRunner: cargo test timeout\n'
    except BaseException as error:
        stdout, stderr, returncode = b'', f'{type(error).__name__}: {error}'.encode(), 125
        interrupted = error
    expected_tests = set(re.findall(r'#\[test\]\s*(?:async\s+)?fn\s+(\w+)', resolve(path, case['assertions']).read_text()))
    observed_tests = set(re.findall(r'^test assertions::(\w+) \.\.\. ok$', stdout.decode(errors='replace'), re.MULTILINE))
    complete = bool(expected_tests) and expected_tests.issubset(observed_tests)
    if returncode == 0 and not complete:
        stderr += ('\nRunner: Frozen assertions did not all pass: ' + ', '.join(sorted(expected_tests - observed_tests)) + '\n').encode()
    (attempt_dir / 'build.stdout').write_bytes(stdout)
    (attempt_dir / 'build.stderr').write_bytes(stderr)
    diagnostics = (stdout + stderr).decode('utf-8', errors='replace')
    save(attempt_dir / 'build.json', {'command': command, 'returncode': returncode,
                                     'duration_seconds': time.monotonic() - start, **classify_build(returncode, diagnostics, expected_tests)})
    verify_snapshot(frozen['snapshot'])
    verify_snapshot(immutable)
    if interrupted is not None:
        raise interrupted
    return returncode == 0 and complete, diagnostics


def verify_model_digest(model, digest):
    with urllib.request.urlopen('http://127.0.0.1:11434/api/tags', timeout=30) as response:
        models = json.loads(response.read())['models']
    matches = [item for item in models if model in (item.get('name'), item.get('model'))]
    normalized = digest.removeprefix('sha256:')
    if len(matches) != 1 or matches[0].get('digest', '').removeprefix('sha256:') != normalized:
        raise RuntimeError('Local model digest does not match frozen protocol')


def infer(path, protocol, model, digest, trial, attempt, body, attempt_dir, frozen):
    # The ledger location is protocol-frozen and shared by all run directories.
    ledger_path = resolve(path, protocol['ledger_file'])
    url = protocol.get('ollama_url', 'http://127.0.0.1:11434/api/chat')
    if url != 'http://127.0.0.1:11434/api/chat':
        raise RuntimeError('Only the reserved local Ollama endpoint is permitted')
    (attempt_dir / 'request.json').write_bytes(body)
    with Ledger(ledger_path) as ledger:
        verify_snapshot(frozen['snapshot'])
        verify_model_digest(model, digest)
        verify_snapshot(frozen['snapshot'])
        call_id = ledger.reserve(model, digest, trial, attempt, hashlib.sha256(body).hexdigest())
        start = time.monotonic()
        try:
            request = urllib.request.Request(url, data=body, headers={'Content-Type': 'application/json'}, method='POST')
            with urllib.request.urlopen(request, timeout=protocol.get('inference_timeout_seconds', 900)) as response:
                raw = response.read()
            (attempt_dir / 'response.json').write_bytes(raw)
            parsed = json.loads(raw)
            save(attempt_dir / 'inference.json', {'call_id': call_id, 'duration_seconds': time.monotonic() - start,
                                                 'prompt_eval_count': parsed.get('prompt_eval_count'),
                                                 'eval_count': parsed.get('eval_count'),
                                                 'total_duration': parsed.get('total_duration'),
                                                 'load_duration': parsed.get('load_duration'),
                                                 'prompt_eval_duration': parsed.get('prompt_eval_duration'),
                                                 'eval_duration': parsed.get('eval_duration')})
            if parsed.get('done') is not True or not isinstance(parsed.get('message', {}).get('content'), str):
                raise RuntimeError('Incomplete or invalid model response')
            answer = parsed['message']['content']
            ledger.finish(call_id, 'response_received')
            return answer
        except BaseException as error:
            save(attempt_dir / 'inference-error.json', {'call_id': call_id, 'status': 'uncertain',
                 'duration_seconds': time.monotonic() - start, 'error': f'{type(error).__name__}: {error}'})
            ledger.finish(call_id, 'uncertain', f'{type(error).__name__}: {error}')
            raise
        finally:
            verify_snapshot(frozen['snapshot'])


def validate_reconciliation(record, frozen_path, events):
    uncertain = {e['call_id'] for e in events if e['event'] == 'terminal' and e['status'] == 'uncertain'}
    if not uncertain:
        return
    if (not record or record.get('frozen_sha256') != sha256_file(frozen_path) or
            record.get('human_approved') is not True or not record.get('approved_by') or
            set(record.get('consumed_call_ids', [])) != uncertain):
        raise RuntimeError('Uncertain calls require explicit bound human reconciliation; no slots are refunded')


def resume_trial(run_dir, model, trial, events, track='unknown'):
    charges = [e for e in events if e['event'] == 'reserve' and e['model'] == model and e['trial'] == trial]
    terminals = {e['call_id']: e for e in events if e['event'] == 'terminal'}
    base = run_dir / re.sub(r'[^A-Za-z0-9_.-]', '_', model) / trial
    records, answer, diagnostics = [], None, None
    budgets = [json.loads(p.read_text()) for p in sorted(base.glob('*/result.json'))]
    budgets = [r for r in budgets if r.get('status') == 'budget_failure']
    for charge in sorted(charges, key=lambda c: c['attempt']):
        directory = base / str(charge['attempt'])
        if not directory.exists():
            raise RuntimeError('Charged trial belongs to another run or retained attempt is missing')
        terminal = terminals.get(charge.get('call_id'), {})
        answer_path = directory / 'answer.txt'
        answer = answer_path.read_text() if answer_path.exists() else answer or ''
        if not answer_path.exists() and (directory / 'response.json').exists():
            try:
                parsed = json.loads((directory / 'response.json').read_bytes())
                content = parsed.get('message', {}).get('content')
                if isinstance(content, str):
                    answer = content
            except (ValueError, UnicodeError, AttributeError, TypeError):
                pass  # Keep corrupt raw bytes untouched; their slot is still charged.
        logs = [directory / f'build.{stream}' for stream in ('stdout', 'stderr')]
        diagnostics = ''.join(log.read_text(errors='replace') for log in logs if log.exists())
        result_path = directory / 'result.json'
        result = json.loads(result_path.read_text()) if result_path.exists() else None
        if not diagnostics:
            error_path = directory / 'inference-error.json'
            diagnostics = (json.loads(error_path.read_text())['error'] if error_path.exists() else
                           (result or {}).get('diagnostics') or terminal.get('detail') or
                           'Runner: previous attempt interrupted before evaluator diagnostics were captured; slot consumed.')
        if result is None:
            original = terminal.get('status', 'reserved')
            result = {'model': model, 'case': trial, 'track': track, 'attempt': charge['attempt'],
                      'call_id': charge.get('call_id'), 'passed': False, 'inference_calls': 1,
                      'status': 'uncertain' if original in ('uncertain', 'reserved') else 'infrastructure',
                      'original_event_status': original, 'diagnostics': diagnostics, 'compile': 'unknown',
                      'behavior': 'incomplete', 'diagnostic': 'incomplete', 'structural_review': 'pending'}
            save(result_path, result)
        result['structural_review'] = 'pending'
        records.append(result)
    records.extend(budgets)
    records.sort(key=lambda r: r.get('attempt', 0))
    if budgets or any(r['passed'] for r in records):
        return 3, answer, diagnostics, records
    return max((e['attempt'] + 1 for e in charges), default=0), answer, diagnostics, records


def run(frozen_path, clearance_path, selected_model=None, controls=False, resume=None, reconciliation=None):
    frozen_path = Path(frozen_path).resolve()
    frozen = json.loads(frozen_path.read_text())
    verify_snapshot(frozen['snapshot'])
    path, protocol, cases = load_protocol(frozen['protocol_path'])
    if not controls:
        validate_clearance(json.loads(Path(clearance_path).read_text()), frozen_path, frozen, protocol)
    models = ['control'] if controls else ([selected_model] if selected_model else list(frozen['model_digests']))
    if any(model not in frozen['model_digests'] for model in models if model != 'control'):
        raise RuntimeError('Model is not in the frozen protocol')
    run_dir = Path(resume).resolve() if resume else resolve(path, protocol['run_root']) / (('control-' if controls else 'run-') + uuid.uuid4().hex)
    events = []
    if resume:
        if controls or json.loads((run_dir / 'frozen.json').read_text()) != frozen:
            raise RuntimeError('Resume requires the original run and exact frozen candidate')
        with Ledger(resolve(path, protocol['ledger_file'])) as ledger:
            events = list(ledger.events)
        record = json.loads(Path(reconciliation).read_text()) if reconciliation else None
        validate_reconciliation(record, frozen_path, events)
        if record:
            save(run_dir / ('reconciliation-' + uuid.uuid4().hex + '.json'), record)
    else:
        run_dir.mkdir(parents=True)
        save(run_dir / 'frozen.json', frozen)
    print(run_dir, flush=True)
    results = []
    for model in models:
        for case in cases:
            answer, diagnostics = None, None
            trial = case['id']
            first_attempt = 0
            if resume:
                first_attempt, answer, diagnostics, previous = resume_trial(run_dir, model, trial, events, case['track'])
                results.extend(previous)
            for attempt in range(first_attempt, 1 if controls else 3):
                attempt_dir = run_dir / re.sub(r'[^A-Za-z0-9_.-]', '_', model) / trial / str(attempt)
                attempt_dir.mkdir(parents=True)
                if controls:
                    code = resolve(path, case['control']).read_text()
                else:
                    try:
                        prompt = prompt_for(path, protocol, case, answer, diagnostics)
                        body = request_body(model, prompt, 8000 if case['track'] == 'reduction_repeat' else 16000)
                    except ValueError as error:
                        record = {'model': model, 'case': trial, 'track': case['track'], 'attempt': attempt,
                                  'passed': False, 'status': 'budget_failure', 'diagnostics': str(error),
                                  'inference_calls': 0, 'compile': 'not_attempted', 'behavior': 'not_measured',
                                  'diagnostic': 'not_measured', 'structural_review': 'pending'}
                        save(attempt_dir / 'result.json', record)
                        results.append(record)
                        verify_snapshot(frozen['snapshot'])
                        break
                    answer = infer(path, protocol, model, frozen['model_digests'][model], trial,
                                   attempt, body, attempt_dir, frozen)
                    (attempt_dir / 'answer.txt').write_text(answer)
                    try:
                        code = extract_code(answer)
                    except ValueError as error:
                        diagnostics = str(error)
                        record = {'model': model, 'case': trial, 'track': case['track'],
                                  'attempt': attempt, 'passed': False, 'diagnostics': diagnostics,
                                  'compile': 'not_attempted', 'behavior': 'not_measured',
                                  'diagnostic': 'not_measured', 'structural_review': 'pending'}
                        save(attempt_dir / 'result.json', record)
                        results.append(record)
                        verify_snapshot(frozen['snapshot'])
                        continue
                passed, diagnostics = build_attempt(path, protocol, case, attempt_dir, code, frozen)
                record = {'model': model, 'case': trial, 'track': case['track'], 'attempt': attempt, 'passed': passed,
                          **{k: v for k, v in json.loads((attempt_dir / 'build.json').read_text()).items()
                             if k in ('compile', 'behavior', 'diagnostic', 'structural_review')}}
                save(attempt_dir / 'result.json', record)
                results.append(record)
                if passed:
                    break
    save(run_dir / ('results-' + uuid.uuid4().hex + '.json' if (run_dir / 'results.json').exists() else 'results.json'), results)
    print(run_dir)
    if controls and (len(results) != len(cases) or any(not r['passed'] for r in results)):
        raise RuntimeError('One or more frozen controls failed; retained diagnostics in run directory')
    return run_dir


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    freeze_parser = commands.add_parser('freeze')
    freeze_parser.add_argument('--protocol', required=True)
    freeze_parser.add_argument('--output', required=True)
    for name in ('control', 'run'):
        sub = commands.add_parser(name)
        sub.add_argument('--frozen', required=True)
        if name == 'run':
            sub.add_argument('--clearance', required=True)
            sub.add_argument('--model')
            sub.add_argument('--resume', help='Retained existing run directory')
            sub.add_argument('--reconciliation', help='Human record consuming uncertain call IDs without refund')
    args = parser.parse_args()
    if args.command == 'freeze':
        freeze(args.protocol, args.output)
    else:
        run(args.frozen, getattr(args, 'clearance', None), getattr(args, 'model', None), args.command == 'control',
            getattr(args, 'resume', None), getattr(args, 'reconciliation', None))


if __name__ == '__main__':
    main()
