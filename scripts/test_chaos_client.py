"""Sensitivity controls for the acceptance runner's refusal-only retries."""
import subprocess
import unittest
from chaos_client import route


class RoutingTests(unittest.TestCase):
    def run_route(self, responses, seconds=3):
        self.calls, self.events = [], []
        clock = [0.0]

        def invoke(candidate, remaining):
            self.calls.append((candidate, remaining))
            clock[0] += 1
            response = next(responses)
            if isinstance(response, Exception):
                raise response
            return response

        return route([(2, "n2"), (3, "n3")], seconds, invoke, self.events.append,
                     now=lambda: clock[0], pause=lambda _: None)

    def test_refusal_routes_to_next_allowed_candidate_with_remaining_budget(self):
        refusal = subprocess.CompletedProcess([], 1, "", "not_leader=true leader_node_id=3\ncommand terminated with exit code 1\n")
        success = subprocess.CompletedProcess([], 0, "applied_term=7\napplied_index=29\n", "")
        self.assertEqual(self.run_route(iter([refusal, success])), success.stdout)
        self.assertEqual(self.calls, [((2, "n2"), 3.0), ((3, "n3"), 2.0)])
        self.assertEqual([e["outcome"] for e in self.events], ["refused", "success"])

    def test_ambiguous_and_mixed_results_are_never_retried(self):
        for stdout, stderr in [("", "client request failed: unconfirmed\n"),
                               ("", "partial_write=true\nnot_leader=true leader_node_id=3\n"),
                               ("applied_index=29\n", "not_leader=true leader_node_id=3\n"),
                               ("", "not_leader=true leader_node_id=3\ntransport error\n")]:
            with self.subTest(stderr=stderr), self.assertRaisesRegex(RuntimeError, "no retry"):
                self.run_route(iter([subprocess.CompletedProcess([], 1, stdout, stderr)]))
            self.assertEqual(len(self.calls), 1)

    def test_timeout_is_unknown_and_terminal(self):
        with self.assertRaisesRegex(RuntimeError, "unknown and was not retried"):
            self.run_route(iter([subprocess.TimeoutExpired("rpc", 3)]))
        self.assertEqual(len(self.calls), 1)
        self.assertEqual(self.events[0]["outcome"], "unknown_timeout")

    def test_refusals_cannot_regain_budget(self):
        refusal = subprocess.CompletedProcess([], 1, "", "not_leader=true leader_node_id=unknown\n")
        with self.assertRaisesRegex(RuntimeError, "deadline exhausted"):
            self.run_route(iter([refusal] * 3), seconds=2)
        self.assertEqual(len(self.calls), 2)
        self.assertEqual([r for _, r in self.calls], [2.0, 1.0])


if __name__ == "__main__":
    unittest.main()
