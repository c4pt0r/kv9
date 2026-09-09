"""Exercise the actual local CLI routing wrapper and its terminal boundaries."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class LocalAdminTests(unittest.TestCase):
    def ready(self, status, dead=False, helper=None):
        with tempfile.TemporaryDirectory(prefix='kv9-root-status.') as name:
            root = Path(name)
            (root / 'n1').mkdir()
            (root / 'template').write_text(status)
            source = root / 'helper.sh'
            source.write_text(helper if helper is not None else Path(__file__).with_name('root_status.sh').read_text())
            return subprocess.run(['bash', '-c', '''
set -euo pipefail
artifact="$1"
source "$artifact/helper.sh"
declare -A node_pids=([1]="$$")
if [[ "$2" == dead ]]; then
  sleep 0 & child=$!
  wait "$child"
  node_pids[1]="$child"
fi
sed "s/CURRENT/${node_pids[1]}/g" "$artifact/template" > "$artifact/n1/status"
node_serving 1
''', 'root-status-test', str(root), 'dead' if dead else 'live'], capture_output=True, text=True)

    def test_readiness_requires_the_current_live_child(self):
        status = 'pid=CURRENT\nbootstrap_state=Serving\nfatal=\n'
        self.assertEqual(self.ready(status).returncode, 0)
        self.assertNotEqual(self.ready(status, dead=True).returncode, 0)

    def test_readiness_rejects_stale_incomplete_duplicate_and_failed_status(self):
        for status in ['pid=1\nbootstrap_state=Serving\nfatal=\n',
                       'pid=CURRENT\nbootstrap_state=Joining\nfatal=\n',
                       'pid=CURRENT\nbootstrap_state=Serving\n',
                       'pid=CURRENT\nbootstrap_state=Serving\nfatal=I/O failure\n',
                       'pid=CURRENT\npid=CURRENT\nbootstrap_state=Serving\nfatal=\n']:
            with self.subTest(status=status):
                self.assertNotEqual(self.ready(status).returncode, 0)

    def test_removing_pid_fence_accepts_the_old_serving_file(self):
        source = Path(__file__).with_name('root_status.sh').read_text()
        anchor = 'pid == expected && '
        self.assertEqual(source.count(anchor), 1)
        stale = 'pid=1\nbootstrap_state=Serving\nfatal=\n'
        self.assertNotEqual(self.ready(stale, helper=source).returncode, 0)
        self.assertEqual(self.ready(stale, helper=source.replace(anchor, '')).returncode, 0)
        self.assertNotEqual(self.ready(stale, helper=source).returncode, 0)

    def call(self, command, responses):
        with tempfile.TemporaryDirectory(prefix='kv9-admin-routing.') as name:
            root = Path(name)
            (root / 'responses.json').write_text(json.dumps(responses))
            binary = root / 'kv9-fixture'
            binary.write_text('#!' + sys.executable + '\n' + '''
import json, sys
from pathlib import Path
root = Path(__file__).resolve().parent
calls = root / 'calls.jsonl'
before = calls.read_text().splitlines() if calls.exists() else []
with calls.open('a') as out: out.write(json.dumps(sys.argv[1:]) + '\\n')
response = json.loads((root / 'responses.json').read_text())[len(before)]
sys.stdout.write(response[1]); sys.stderr.write(response[2]); sys.exit(response[0])
''')
            binary.chmod(0o700)
            evidence = root / 'routing.jsonl'
            result = subprocess.run([
                sys.executable, str(Path(__file__).with_name('local_admin_client.py')),
                '--bin', str(binary), '--addresses', '2=n2,3=n3', '--timeout', '3',
                '--evidence', str(evidence), '--', *command,
            ], capture_output=True, text=True, timeout=5)
            calls = root / 'calls.jsonl'
            return (result, [json.loads(line) for line in calls.read_text().splitlines()] if calls.exists() else [],
                    [json.loads(line) for line in evidence.read_text().splitlines()] if evidence.exists() else [])

    def test_membership_calls_route_exclusive_refusal_and_preserve_receipt(self):
        for command in ['admit-node', 'promote-node']:
            with self.subTest(command=command):
                response = 'applied_term=4\napplied_index=21\njoin_ticket=fixture-ticket\n'
                result, calls, events = self.call([command, '--node-id', '4'], [
                    [1, '', 'not_leader=true leader_node_id=3\n'], [0, response, '']])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, response)
                self.assertEqual(calls, [['client', command, '--addr', node, '--node-id', '4'] for node in ['n2', 'n3']])
                self.assertEqual([e['outcome'] for e in events], ['refused', 'success'])
                self.assertNotIn('fixture-ticket', json.dumps(events))
                self.assertGreater(events[0]['remaining'], events[1]['remaining'])

    def test_ambiguous_membership_outcomes_never_issue_another_cli_call(self):
        for stdout, stderr in [('', 'client request failed: response lost\n'),
                               ('', 'not leader; try node 3\n'),
                               ('applied_index=21\n', 'not_leader=true leader_node_id=3\n')]:
            with self.subTest(stderr=stderr):
                result, calls, events = self.call(['admit-node', '--node-id', '4'], [[1, stdout, stderr]])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('no retry', result.stderr)
                self.assertEqual(len(calls), 1)
                self.assertEqual(events[0]['outcome'], 'terminal')

    def test_wrapper_refuses_unsupported_operations_or_address_override(self):
        for command in [['raw-put'], ['create-keyspace'], ['admit-node', '--addr', 'unconfigured']]:
            with self.subTest(command=command):
                result, calls, events = self.call(command, [])
                self.assertEqual(result.returncode, 2)
                self.assertEqual(calls, [])
                self.assertEqual(events, [])


if __name__ == '__main__':
    unittest.main()
