"""Prepare actual fixture directories before constructing a root descriptor."""
import re


def prepare_stores(run, binary, directories):
    result = []
    for node, directory in directories.items():
        output = run([binary, 'store-prepare', '--node-id', node, '--data-dir', directory])
        fields = dict(field.split('=', 1) for field in output.split())
        identity = fields.get('store_incarnation', '')
        if fields.get('store_prepared') != 'true' or fields.get('node_id') != str(node) or not re.fullmatch('[0-9a-f]{32}', identity):
            raise ValueError('fixture store preparation returned an invalid identity')
        result.append(f'{node}={identity}')
    return ','.join(result)
