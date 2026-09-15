#!/usr/bin/env python3
"""Deterministic storage counters only; no Docker, cluster, or payload workload."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

SPEC = importlib.util.spec_from_file_location(
    "checkpoint_chaos_guard", Path(__file__).with_name("checkpoint-publication-chaos.py"))
HELPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPER)
GIB, MIB = HELPER.GIB, HELPER.MIB
LAUNCH = 9*GIB + 8*MIB


class FakeFilesystems:
    def __init__(self, shared=False, host=12*GIB, output=12*GIB):
        self.devices = {"/": 1, "/output": 1 if shared else 2, "/prior": 1 if shared else 2}
        self.free = {1: host, 2: output}
        self.samples = []

    def stat(self, path):
        return SimpleNamespace(st_dev=self.devices[str(path)])

    def statvfs(self, path):
        device = self.devices[str(path)]
        self.samples.append(device)
        # Exercise f_bavail conversion, with no f_bfree/reserved-block credit.
        return SimpleNamespace(f_bavail=self.free[device], f_frsize=1)

    def budget(self):
        return HELPER.FilesystemBudget("/output", stat_fn=self.stat, statvfs_fn=self.statvfs)


class StorageGuardTests(unittest.TestCase):
    def test_split_filesystems_sum_independent_growth(self):
        fs = FakeFilesystems()
        budget = fs.budget()
        fs.free[1] -= 400*MIB
        fs.free[2] -= 500*MIB
        sample = budget.sample()
        budget.check(launch=True)
        self.assertEqual(sample["maximum_observed_total_decrease_bytes"], 900*MIB)
        self.assertEqual([r["roles"] for r in sample["filesystems"]], [["host"], ["output"]])
        fs.free[2] -= 125*MIB
        budget.sample()
        with self.assertRaisesRegex(RuntimeError, "total conservative"):
            budget.check()

    def test_shared_filesystem_is_counted_and_sampled_once(self):
        fs = FakeFilesystems(shared=True)
        budget = fs.budget()
        fs.samples.clear()
        fs.free[1] -= 900*MIB
        sample = budget.sample()
        budget.check()
        self.assertEqual(fs.samples, [1])
        self.assertEqual(len(sample["filesystems"]), 1)
        row = sample["filesystems"][0]
        self.assertEqual(row["roles"], ["host", "output"])
        self.assertEqual(sample["maximum_observed_total_decrease_bytes"], 900*MIB)
        self.assertEqual(row["baseline_available_bytes"] - row["minimum_available_bytes"], 900*MIB)

    def test_cleanup_never_erases_prior_growth_or_offsets_other_filesystem(self):
        fs = FakeFilesystems()
        budget = fs.budget()
        fs.free[1] -= 600*MIB
        budget.sample()
        fs.free[1] = 15*GIB
        fs.free[2] -= 500*MIB
        sample = budget.sample()
        self.assertEqual(budget.host["current_available_bytes"], 15*GIB)
        self.assertEqual(budget.host["minimum_available_bytes"], 12*GIB - 600*MIB)
        self.assertEqual(sample["maximum_observed_total_decrease_bytes"], 1100*MIB)
        with self.assertRaisesRegex(RuntimeError, "total conservative"):
            budget.check()

    def test_host_floor_failure_is_independent_of_large_output_space(self):
        fs = FakeFilesystems(host=LAUNCH, output=100*GIB)
        budget = fs.budget()
        fs.free[1] = 8*GIB - 1
        budget.sample()
        with self.assertRaisesRegex(RuntimeError, "floor: host"):
            budget.check()

    def test_output_launch_headroom_and_runtime_floor(self):
        fs = FakeFilesystems(host=100*GIB, output=LAUNCH - 1)
        budget = fs.budget()
        with self.assertRaisesRegex(RuntimeError, "launch headroom: output"):
            budget.check(launch=True)
        fs.free[2] = 8*GIB - 1
        budget.sample()
        with self.assertRaisesRegex(RuntimeError, "floor: output"):
            budget.check()

    def test_exact_launch_and_added_byte_boundaries(self):
        fs = FakeFilesystems(host=LAUNCH, output=LAUNCH)
        budget = fs.budget()
        budget.check(launch=True)
        fs.free[1] -= GIB
        budget.sample()
        budget.check()
        fs.free[2] -= 1
        budget.sample()
        with self.assertRaisesRegex(RuntimeError, "total conservative"):
            budget.check()

    def test_device_change_refuses_before_sampling_replacement(self):
        fs = FakeFilesystems()
        budget = fs.budget()
        fs.devices["/output"] = 3
        fs.samples.clear()
        with self.assertRaisesRegex(RuntimeError, "device changed: output"):
            budget.sample()
        self.assertEqual(fs.samples, [])

    def test_legacy_same_filesystem_failure_retains_original_minimum(self):
        fs = FakeFilesystems(shared=True, host=20*GIB)
        budget = fs.budget()
        budget.inherit({"baseline_available_bytes": 12*GIB,
                        "minimum_available_bytes": 12*GIB - 600*MIB}, Path("/prior"))
        budget.check(launch=True)
        self.assertEqual(budget.snapshot()["maximum_observed_total_decrease_bytes"], 600*MIB)
        self.assertEqual(budget.host["baseline_available_bytes"], 12*GIB)

    def test_legacy_failure_cannot_seed_split_filesystem_baselines(self):
        fs = FakeFilesystems()
        budget = fs.budget()
        with self.assertRaisesRegex(RuntimeError, "legacy failure needs"):
            budget.inherit({"baseline_available_bytes": 12*GIB,
                            "minimum_available_bytes": 11*GIB}, Path("/prior"))

    def test_new_failure_preserves_both_high_water_counters(self):
        fs = FakeFilesystems()
        original = fs.budget()
        fs.free[1] -= 400*MIB
        fs.free[2] -= 300*MIB
        prior = {"filesystem_budget": original.sample()}
        fs.free[1] = fs.free[2] = 20*GIB
        resumed = fs.budget()
        resumed.inherit(prior, Path("/prior"))
        resumed.check(launch=True)
        self.assertEqual(resumed.snapshot()["maximum_observed_total_decrease_bytes"], 700*MIB)


if __name__ == "__main__":
    unittest.main(verbosity=2)
