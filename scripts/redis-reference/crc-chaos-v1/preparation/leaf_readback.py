#!/usr/bin/env python3
"""Additional read-only leaf check; does not replace the unchanged full audit."""
import argparse
import hashlib
import json
from pathlib import Path
import re
from contract import require


def netem_leaf(text):
    blocks = [s for s in re.split(r'(?=^qdisc )', text, flags=re.M) if s.startswith('qdisc netem ')]
    require(len(blocks) == 1, 'one selected netem leaf required')
    block = blocks[0]
    identity = re.match(r'qdisc netem (\S+) parent (\S+)', block)
    loss = re.search(r'\bloss (\d+(?:\.\d+)?)%(?: (\d+(?:\.\d+)?)%)?', block)
    dropped = re.search(r'\(dropped (\d+),', block)
    require(identity and loss and dropped, 'netem identity/loss/drop fields missing')
    require(float(loss[1]) == 30 and (loss[2] is None or float(loss[2]) == 0), 'not uncorrelated 30% loss')
    return {'handle': identity[1], 'parent': identity[2], 'dropped': int(dropped[1])}


def leaf_delta(before, after):
    a, b = netem_leaf(before), netem_leaf(after)
    require((a['handle'], a['parent']) == (b['handle'], b['parent']), 'netem leaf identity changed')
    headers = [[x for x in text.splitlines() if x.startswith('qdisc netem ')] for text in (before, after)]
    require(headers[0] == headers[1], 'netem configuration/generation header changed')
    require(b['dropped'] > a['dropped'], 'selected netem leaf has no new drops')
    return {'before': a, 'after': b, 'netem_leaf_drop_delta': b['dropped'] - a['dropped']}


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def readback(artifact):
    artifact = Path(artifact)
    load = lambda name: json.loads((artifact / name).read_text())
    plan = load('executed-plan.json')
    summary = load('summary.json')
    require(summary['accepted'] is True and summary['cleanup']['complete'] is True, 'fixture not complete')
    rows = load('commands/commands.json')
    effects = {x['name']: x for x in load('packet-effects.json')}
    evidence = []
    identities = []
    def raw(index):
        require(type(index) is int and 0 <= index < len(rows), 'invalid command index')
        row = rows[index]
        require(row['exit_code'] == 0 and row['started_unix_ns'] < row['ended_unix_ns'], 'failed/unordered command')
        path = artifact / 'commands' / row['stdout']
        return row, path.read_text(), str(path), sha(path)
    for suffix in ('before', 'after'):
        effect = effects['vip-partial-loss-' + suffix]
        selected = effect['pods']['kv9-native-batch-client']['qdisc']
        i = selected['command']
        row, text, path, digest = raw(i)
        require(text == selected['text'], 'qdisc raw bytes differ')
        pod_row, pod_text, _, _ = raw(i - 3)
        cri_row, cri_text, _, _ = raw(i - 2)
        stat_row, stat_text, _, _ = raw(i - 1)
        pod, cri = json.loads(pod_text), json.loads(cri_text)
        require(pod_row['command'] == ['kubectl', '--kubeconfig', plan['kubeconfig'], '--request-timeout=15s', 'get', 'pod', 'kv9-native-batch-client', '-n', plan['namespace'], '-o', 'json'], 'wrong selected Pod query')
        require(pod['metadata']['labels']['app'] == 'kv9-native-batch-client', 'wrong Pod role')
        status = pod['status']['containerStatuses']
        require(len(status) == 1 and status[0]['restartCount'] == 0, 'native Pod container changed')
        cid = status[0]['containerID'].removeprefix('containerd://')
        pid = cri['info']['pid']
        require(type(pid) is int and pid > 0, 'invalid netns PID')
        require(cri_row['command'] == ['docker', 'exec', plan['node'], 'crictl', 'inspect', cid] and cri['status']['id'] == cid, 'CRI binding differs')
        require(stat_row['command'] == ['docker', 'exec', plan['node'], 'cat', f'/proc/{pid}/stat'], 'wrong stat PID')
        require(stat_text.split(' ', 1)[0] == str(pid), 'stat content PID differs')
        start = stat_text.rsplit(')', 1)[1].split()[19]
        require(start.isdecimal(), 'invalid start ticks')
        require(row['command'] == ['docker', 'exec', plan['node'], 'nsenter', '-t', str(pid), '-n', 'tc', '-s', '-d', 'qdisc', 'show', 'dev', 'eth0'], 'qdisc command is not selected netns')
        require(effect['started_ns'] <= row['started_unix_ns'] < row['ended_unix_ns'] <= effect['ended_ns'], 'qdisc outside retained effect envelope')
        require(all(rows[j]['ended_unix_ns'] <= rows[j+1]['started_unix_ns'] for j in range(i-3, i)), 'binding commands not serial')
        identities.append((pod['metadata']['uid'], cid, pid, start))
        evidence.append({'command': i, 'path': path, 'sha256': digest, 'started_ns': row['started_unix_ns'], 'ended_ns': row['ended_unix_ns'], 'text': text})
    require(identities[0] == identities[1], 'selected netns lifetime changed')
    require(evidence[0]['ended_ns'] < evidence[1]['started_ns'], 'leaf snapshots out of order')
    result = leaf_delta(evidence[0]['text'], evidence[1]['text'])
    for row in evidence: del row['text']
    result.update(accepted=True, revision=plan['revision'], artifact=str(artifact), identity=identities[0], raw=evidence,
                  scope='Additional exact same-netem-leaf positive delta only; unchanged full history/effect/lifecycle audit is separately mandatory',
                  inputs={name: sha(artifact/name) for name in ('executed-plan.json', 'summary.json', 'packet-effects.json', 'commands/commands.json')})
    return result


def main():
    parser = argparse.ArgumentParser(); parser.add_argument('--artifact', type=Path, required=True); parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(); require(not args.output.exists(), 'output must be new')
    result = readback(args.artifact)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    print('PASS: bound netem leaf delta', result['netem_leaf_drop_delta'])


if __name__ == '__main__': main()
