#!/usr/bin/env python3
"""Reject isolated routing faults with actual delivery and ownership regressions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GRPC = 'crates/raft/src/grpc.rs'
BODY = 'crates/raft/src/grpc/direct_body.rs'
BODY_TESTS = 'crates/raft/src/grpc/direct_body/tests.rs'
TCP = 'crates/raft/src/transport.rs'


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def replace_once(text, before, after):
    if text.count(before) != 1 or before == after:
        raise RuntimeError('source control anchor is not unique')
    return text.replace(before, after)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = {p: (ROOT / p).read_text() for p in (GRPC, BODY, BODY_TESTS, TCP)}
    grpc, body, tcp = sources[GRPC], sources[BODY], sources[TCP]
    cases = [
        ('missing-route-notification', GRPC, replace_once(grpc,
         'sender.destination.send_replace(peer.destination.clone());', 'let _ = sender;'),
         'grpc::tests::registered_address_change_replaces_live_peer_stream', 'new configured endpoint received no traffic'),
        ('address-reuse-admits-old-generation', BODY, replace_once(body,
         'if Arc::ptr_eq(&message.destination, &self.destination) {',
         'if message.destination.addr == self.destination.addr {'),
         'grpc::direct_body::tests::route_generation_filter_rejects_old_queue_entries_even_after_address_reuse', 'assertion `left == right` failed'),
        ('replace-live-worker-on-every-send', GRPC, replace_once(grpc,
         '.is_none_or(|sender| sender.task.is_finished())', '.is_none_or(|_| true)'),
         'grpc::tests::route_updates_keep_one_owned_worker_and_same_address_is_idempotent', 'route update spawned another worker'),
        ('tcp-drop-retains-listener', TCP, replace_once(tcp,
         'impl Drop for TcpTransport {\n    fn drop(&mut self) {\n        self.shutdown();',
         'impl Drop for TcpTransport {\n    fn drop(&mut self) {\n        // Detach the listener.'),
         'transport::tests::dropping_tcp_transport_closes_its_listener', 'dropped TCP transport retained its listener'),
        ('tcp-retains-old-connection', TCP, replace_once(tcp,
         'if peer.addr != addr {\n            peer.stream = None;', 'if peer.addr != addr {\n            // Keep the stale stream.'),
         'transport::tests::tcp_address_update_moves_delivery_and_shutdown_drops_connections', 'new endpoint received no post-update message'),
        ('stale-body-polls-replacement-queue', BODY, replace_once(body,
         'if !owns(&state, &self.token) {\n            coop.made_progress();',
         'if !state.receiver_open {\n            coop.made_progress();'),
         'grpc::direct_body::tests::same_route_reconnect_fences_retained_body_and_its_drop',
         'assertion failed: matches!(poll_body(&mut old_body, &intruder_waker), Poll::Ready(None))'),
        ('stale-drop-invalidates-replacement', BODY, replace_once(body,
         'if !owns(&state, token) {', 'if !state.receiver_open {'),
         'grpc::direct_body::tests::same_route_reconnect_fences_retained_body_and_its_drop',
         'assertion failed: owns(&rx.0.state.lock().unwrap(), &new_session.token)'),
        ('enqueue-postpones-stall-deadline', BODY, replace_once(body,
         'if newly_pending {\n                state.pending_since = Some(Instant::now());\n            }',
         'state.pending_since = Some(Instant::now());'),
         'grpc::direct_body::tests::sends_stale_inspection_and_notifications_do_not_reset_stall_age',
         'stale traffic reset the backlog age'),
        ('ready-body-restores-cooperative-budget', BODY, replace_once(body,
         'if inspected != 0 || result.is_ready() {\n            coop.made_progress();\n        }',
         'let _ = inspected; // Incorrectly restore the budget after ready work.'),
         'grpc::direct_body::tests::ready_batches_exhaust_task_budget_without_dequeueing_on_pending',
         'repeated Ready batches bypassed Tokio\'s cooperative budget'),
        ('producer-notifies-idle-watchdog', BODY, replace_once(body,
         'if newly_pending {\n                state.pending_since = Some(Instant::now());\n            }',
         'if newly_pending {\n                state.pending_since = Some(Instant::now());\n                self.0.changed.notify_one();\n            }'),
         'grpc::direct_body::tests::producer_wakes_body_but_not_idle_watchdog_across_empty_transitions',
         'producer woke the independent idle watchdog'),
        ('idle-watchdog-has-no-timer', BODY, replace_once(body,
         '_ = tokio::time::sleep_until(deadline) => {',
         '_ = tokio::time::sleep_until(deadline), if self.shared.state.lock().unwrap().pending_since.is_some() => {'),
         'grpc::direct_body::tests::real_unpolled_backlog_after_idle_arming_expires_without_producer_notify',
         'real unpolled backlog missed its unchanged progress deadline'),
        ('idle-timer-declares-empty-queue-stalled', BODY, replace_once(body,
         'state.pending_since.is_some_and(|at| Instant::now() >= at + STREAM_PROGRESS_BUDGET)',
         'state.pending_since.is_none_or(|at| Instant::now() >= at + STREAM_PROGRESS_BUDGET)'),
         'grpc::direct_body::tests::persistent_idle_watchdog_checks_twice_despite_repeated_polls_without_reconnect',
         'healthy persistent idle watchdog requested reconnect'),
        ('sender-close-omits-watchdog-notification', BODY, replace_once(body,
         '        self.0.changed.notify_one();\n    }\n}\n\nimpl Receiver {',
         '        // Incorrectly leave the idle watchdog asleep on sender closure.\n    }\n}\n\nimpl Receiver {'),
         'grpc::direct_body::tests::idle_sender_receiver_and_body_closure_notify_without_waiting_for_timer',
         'lifecycle closure lost its prompt watchdog notification: sender'),
        ('old-timer-ignores-current-backlog-progress', BODY, replace_once(body,
         'state.pending_since.is_some_and(|at| Instant::now() >= at + STREAM_PROGRESS_BUDGET)',
         'Instant::now() >= deadline'),
         'grpc::direct_body::tests::valid_batch_progress_resets_armed_budget_and_empty_queue_clears_it',
         'an already-armed old deadline ignored real dequeue progress'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    manifest = dict(sources={p: digest(s) for p, s in sources.items()}, controls=[])
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    for p, s in sources.items():
        target = output / 'original' / p
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(s)
    with tempfile.TemporaryDirectory(prefix='kv9-route-controls.') as directory:
        tree = Path(directory)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, path, mutant, test, failure in cases:
            folder = output / name
            folder.mkdir()
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, path=path, test=test, expected_failure=failure,
                        mutant_sha256=digest(mutant), runs=[])
            for phase, text in [('baseline', sources[path]), ('mutant', mutant), ('restored', sources[path])]:
                for p, s in sources.items():
                    (tree / p).write_text(text if p == path else s)
                expected_sources = {p: digest(text if p == path else s) for p, s in sources.items()}
                command = ['cargo', 'test', '--locked', '-p', 'kv9-raft', '--lib', test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one compiled test')
                if phase == 'mutant':
                    if result.returncode != 101 or failure not in result.stdout or '0 passed; 1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                actual_sources = {p: digest((tree / p).read_text()) for p in sources}
                if actual_sources != expected_sources:
                    raise RuntimeError('isolated source changed during a control')
                case['runs'].append(dict(phase=phase, command=command, exit_code=result.returncode, sources=actual_sources))
            manifest['controls'].append(case)
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    if any((ROOT / p).read_text() != text for p, text in sources.items()):
        raise RuntimeError('source changed during controls')
    print(f'PASS: {len(cases)} isolated route ownership source controls checked', flush=True)


if __name__ == '__main__':
    main()
