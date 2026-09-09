"""Regression controls for child failure and bounded progress observation."""
import unittest
from unittest.mock import Mock, patch

from workload_e2e_support import wait


class ProgressTests(unittest.TestCase):
    def test_failed_workload_is_reported_without_waiting_for_progress(self):
        condition = Mock(return_value=False)
        with patch("workload_e2e_support.time.sleep") as sleep:
            with self.assertRaisesRegex(RuntimeError, "performance-stop exited 1.*performance-stop.log"):
                wait("measured progress", condition, {}, workload=("performance-stop", Mock(poll=lambda: 1)))
        condition.assert_not_called()
        sleep.assert_not_called()

    def test_stale_progress_cannot_hide_even_a_successful_child_exit(self):
        condition = Mock(return_value=True)
        with self.assertRaisesRegex(RuntimeError, "killed-generator exited 0"):
            wait("progress before kill", condition, {}, workload=("killed-generator", Mock(poll=lambda: 0)))
        condition.assert_not_called()

    def test_live_child_progress_is_returned(self):
        self.assertEqual(wait("progress", lambda: "ready", {1: Mock(poll=lambda: None)},
                              workload=("workload", Mock(poll=lambda: None))), "ready")

    def test_replica_exit_cannot_be_hidden_by_progress(self):
        with self.assertRaisesRegex(RuntimeError, "replica 2 exited"):
            wait("progress", lambda: True, {2: Mock(poll=lambda: -9)})

    def test_wait_uses_one_fixed_deadline(self):
        with patch("workload_e2e_support.time.monotonic", side_effect=[0, 0, 1, 2]), \
                patch("workload_e2e_support.time.sleep") as sleep:
            with self.assertRaisesRegex(RuntimeError, "timed out: progress"):
                wait("progress", lambda: False, {}, seconds=2, workload=("workload", Mock(poll=lambda: None)))
        self.assertEqual(sleep.call_count, 2)


if __name__ == "__main__":
    unittest.main()
