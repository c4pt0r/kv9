#!/usr/bin/env python3
"""Check source/evidence inventory bounds without Cargo or external services."""
import hashlib
import importlib.util
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

    def test_archive_allowance_does_not_extend_other_paths(self):
        for name in ['source.rs', 'docs/other.tar.gz', 'data/original-evidence.tar.gz']:
            with self.subTest(name=name):
                self.names = []
                self.file(name, b'a' * (BUILDER.MAX_SOURCE_FILE + 1))
                self.rejected()

    def test_archive_file_bound_is_enforced(self):
        self.file('docs/capture/original-evidence.tar.gz', b'a' * 129)
        with patch.object(BUILDER, 'MAX_EVIDENCE_FILE', 128):
            self.rejected()

    def test_source_and_archive_aggregate_bounds_are_independent(self):
        self.file('source.rs', b'a' * 128)
        self.file('docs/a/original-evidence.tar.gz', b'b' * 128)
        with patch.object(BUILDER, 'MAX_SOURCE_TOTAL', 128), patch.object(BUILDER, 'MAX_EVIDENCE_TOTAL', 128):
            self.assertEqual(len(BUILDER.snapshot()['sources']), 2)
            source = self.file('extra.rs', b'c')
            self.rejected()
            source.unlink()
            self.names.remove('extra.rs')
            self.file('docs/b/original-evidence.tar.gz', b'c')
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
        self.names = ['deleted.rs']
        self.assertEqual(BUILDER.snapshot()['sources'], {'deleted.rs': None})


if __name__ == '__main__':
    unittest.main(verbosity=2)
