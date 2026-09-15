#!/usr/bin/env python3
"""Check source/documentation inventory bounds without Cargo or external services."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('inventory_under_test', ROOT / 'scripts/build-workload.py')
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


class SourceInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='kv9-source-inventory-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.names = []
        self.root_patch = patch.object(BUILDER, 'ROOT', self.root)
        self.root_patch.start()
        self.addCleanup(self.root_patch.stop)
        self.git_patch = patch.object(BUILDER.subprocess, 'check_output', side_effect=self.git)
        self.git_patch.start()
        self.addCleanup(self.git_patch.stop)

    def git(self, argv, **kwargs):
        self.assertEqual(kwargs['cwd'], self.root)
        if argv[:2] == ['git', 'ls-files']:
            self.assertEqual(argv, ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'])
            return ('\0'.join(self.names) + '\0').encode()
        if argv == ['git', 'rev-parse', 'HEAD']:
            return 'a' * 40 + '\n'
        if argv == ['git', 'status', '--porcelain', '--untracked-files=all']:
            return b' M source.rs\n'
        self.fail(f'unexpected command: {argv!r}')

    def file(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        self.names.append(name)
        return path

    def rejected(self, message='source inventory exceeds its byte bound'):
        with self.assertRaisesRegex(RuntimeError, '^' + message + '$'):
            BUILDER.snapshot()

    def test_archive_is_fully_hashed_and_byte_changes_change_identity(self):
        archive = self.file('docs/capture/original-evidence.tar.gz', b'a' * (BUILDER.MAX_SOURCE_FILE + 1))
        self.file('source.rs', b'fn main() {}\n')
        first = BUILDER.snapshot()
        self.assertEqual(first['sources'], {name: hashlib.sha256((self.root / name).read_bytes()).hexdigest()
                                            for name in self.names})
        with archive.open('r+b') as stream:
            stream.seek(BUILDER.MAX_SOURCE_FILE)
            stream.write(b'b')
        second = BUILDER.snapshot()
        self.assertNotEqual(first['sources'][self.names[0]], second['sources'][self.names[0]])
        self.assertEqual(first['sources']['source.rs'], second['sources']['source.rs'])
        self.assertEqual(second['revision'], 'a' * 40)
        self.assertTrue(second['dirty'])

    def test_ordinary_source_exact_limit_is_accepted(self):
        self.file('source.rs', b'a' * BUILDER.MAX_SOURCE_FILE)
        self.assertEqual(len(BUILDER.snapshot()['sources']), 1)

    def test_documentation_allowance_does_not_extend_runtime_or_unknown_formats(self):
        for name in ['source.rs', 'crates/raft/large.json', 'scripts/large.json',
                     'data/original-evidence.tar.gz', 'docs-other/large.json',
                     'docs/source.rs', 'docs/helper.py', 'docs/opaque.bin',
                     'docs/archive.gz', 'docs/archive.tar.gz.001.rs',
                     'scripts/corpus.bin.gz', 'docs/corpus.bin.gz.py']:
            with self.subTest(name=name):
                self.names = []
                self.file(name, b'a' * (BUILDER.MAX_SOURCE_FILE + 1))
                self.rejected()

    def test_documentation_formats_are_fully_hashed_with_exact_file_boundary(self):
        for name in ['docs/report.md', 'docs/capture/input-inventory.json',
                     'docs/capture/original-evidence.tar.gz', 'docs/capture/evidence.tar.gz',
                     'docs/capture/evidence.tar.gz.001', 'docs/capture/cargo.jsonl',
                     'docs/capture/samples.csv', 'docs/capture/run.stderr',
                     'docs/capture/corpus.bin.gz']:
            with self.subTest(name=name):
                self.names = []
                path = self.file(name, b'a' * 128)
                with patch.object(BUILDER, 'MAX_SOURCE_FILE', 64), \
                        patch.object(BUILDER, 'MAX_DOCUMENTATION_FILE', 128):
                    first = BUILDER.snapshot()
                    self.assertEqual(first['sources'][name], hashlib.sha256(b'a' * 128).hexdigest())
                    path.write_bytes(b'a' * 127 + b'b')
                    second = BUILDER.snapshot()
                    self.assertNotEqual(first['sources'][name], second['sources'][name])
                    path.write_bytes(b'a' * 129)
                    self.rejected()

    def test_documentation_path_class_is_explicit(self):
        for name in ['docs/../crates/raft/large.json', '/docs/large.json',
                     'crates/docs/large.json', 'docs/archive.tar.gz.bad',
                     'docs/archive.tar.gz.0000', 'docs/Cargo.toml']:
            with self.subTest(name=name):
                self.assertFalse(BUILDER.documentation_input(name))

    def test_source_and_documentation_aggregate_bounds_are_independent(self):
        self.file('source.rs', b'a' * 128)
        self.file('docs/a/original-evidence.tar.gz', b'b' * 128)
        with patch.object(BUILDER, 'MAX_SOURCE_TOTAL', 128), patch.object(BUILDER, 'MAX_DOCUMENTATION_TOTAL', 128):
            self.assertEqual(len(BUILDER.snapshot()['sources']), 2)
            source = self.file('extra.rs', b'c')
            self.rejected()
            source.unlink()
            self.names.remove('extra.rs')
            self.file('docs/b/input-inventory.json', b'c')
            self.rejected()

    def test_documentation_code_still_consumes_the_ordinary_aggregate(self):
        self.file('source.rs', b'a' * 128)
        self.file('docs/helper.py', b'b')
        with patch.object(BUILDER, 'MAX_SOURCE_TOTAL', 128):
            self.rejected()

    def test_compressed_corpus_consumes_the_documentation_aggregate(self):
        self.file('docs/corpus.bin.gz', b'a' * 128)
        with patch.object(BUILDER, 'MAX_SOURCE_FILE', 64), \
                patch.object(BUILDER, 'MAX_DOCUMENTATION_TOTAL', 128):
            self.assertEqual(len(BUILDER.snapshot()['sources']), 1)
            self.file('docs/extra.json', b'b')
            self.rejected()

    def test_live_and_dangling_symlinks_are_rejected(self):
        self.file('source.rs', b'a')
        link = self.root / 'link.rs'
        self.names.append('link.rs')
        for target in ['source.rs', 'missing.rs']:
            with self.subTest(target=target):
                link.symlink_to(target)
                self.rejected('source inventory requires regular files')
                link.unlink()

    def test_tracked_deletion_remains_part_of_identity(self):
        self.names = ['deleted.rs', 'docs/deleted.json']
        self.assertEqual(BUILDER.snapshot()['sources'], {'deleted.rs': None, 'docs/deleted.json': None})


class CurrentRepositoryInventoryTests(unittest.TestCase):
    def test_current_repository_preserves_every_listed_input(self):
        original = BUILDER.subprocess.check_output
        listed = []

        def observe(argv, **kwargs):
            result = original(argv, **kwargs)
            if argv[:2] == ['git', 'ls-files']:
                listed.extend(result.decode().strip('\0').split('\0'))
            return result

        with patch.object(BUILDER.subprocess, 'check_output', side_effect=observe):
            snapshot = BUILDER.snapshot()
        self.assertEqual(set(snapshot['sources']), set(listed))
        self.assertTrue(snapshot['sources'])
        self.assertEqual(BUILDER.MAX_SOURCE_FILE, 2 * 1024 * 1024)
        self.assertEqual(BUILDER.MAX_SOURCE_TOTAL, 64 * 1024 * 1024)
        counts = {'ordinary': 0, 'documentation': 0}
        totals = dict(counts)
        for name, digest in snapshot['sources'].items():
            if digest is None:
                continue
            self.assertRegex(digest, r'^[0-9a-f]{64}$')
            kind = 'documentation' if BUILDER.documentation_input(name) else 'ordinary'
            counts[kind] += 1
            totals[kind] += (ROOT / name).stat().st_size
        print(json.dumps({'current_repository': str(ROOT), 'revision': snapshot['revision'],
                          'dirty': snapshot['dirty'], 'listed_inputs': len(snapshot['sources']),
                          'counts': counts, 'bytes': totals}, sort_keys=True))


if __name__ == '__main__':
    unittest.main(verbosity=2)
