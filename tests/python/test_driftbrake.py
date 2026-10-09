"""
Integration tests for driftbrake Python bindings.
"""

import unittest
import sys
import os

# Add python source path
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../driftbrake-py/python")))

import driftbrake as db


class TestDriftbrakePython(unittest.TestCase):
    def test_reconcile_history_appends(self):
        history = db.ReconcileHistory()
        self.assertEqual(len(history), 0)
        self.assertEqual(history.total_reverts(), 0)

        history.append(100, 90)
        history.append(100, 80)
        self.assertEqual(len(history), 2)
        self.assertEqual(history.recent_ratios(2), [0.9, 0.8])

    def test_revert_streak_and_reset(self):
        history = db.ReconcileHistory()
        rev_hash = b"\x01" * 32

        history.record_revert(rev_hash, 10, 21000, 1000)
        self.assertEqual(history.consecutive_reverts(), 1)
        history.record_revert(rev_hash, 11, 21000, 1000)
        self.assertEqual(history.consecutive_reverts(), 2)

        # Successful trade resets consecutive reverts
        history.append(100, 95)
        self.assertEqual(history.consecutive_reverts(), 0)

    def test_fast_guard_halt(self):
        policy = db.ReconcilePolicy.default_dual_guard()
        history = db.ReconcileHistory()

        for _ in range(5):
            history.append(100, 100)

        dec = policy.evaluate(history)
        self.assertFalse(dec.should_halt)

        # 3 acute underperformances
        for _ in range(3):
            history.append(100, 10)

        dec = policy.evaluate(history)
        self.assertTrue(dec.should_halt)
        self.assertEqual(dec.reason_name, "FastGuard")

        with self.assertRaises(db.StrategyHaltedError):
            dec.unwrap_or_raise()

    def test_institutional_revert_guard(self):
        policy = db.ReconcilePolicy.institutional(revert_burst_limit=3)
        history = db.ReconcileHistory()
        rev_hash = b"\x02" * 32

        history.record_revert(rev_hash, 100, 21000, 1000)
        history.record_revert(rev_hash, 101, 21000, 1000)
        self.assertFalse(policy.evaluate(history).should_halt)

        history.record_revert(rev_hash, 102, 21000, 1000)
        dec = policy.evaluate(history)
        self.assertTrue(dec.should_halt)
        self.assertEqual(dec.reason_name, "RevertBurst")

    def test_vectorized_sweep(self):
        trades = {
            "predicted": [100] * 30,
            "realized": [95] * 20 + [20] * 10,
            "is_revert": [False] * 30,
        }
        results = db.run_sweep(
            trades,
            fast_thresholds=[0.3, 0.5],
            slow_thresholds=[0.7, 0.8],
            revert_limits=[2, 3],
        )
        self.assertGreater(len(results), 0)
        first = results[0]
        self.assertGreaterEqual(first.total_halts, 1)


if __name__ == "__main__":
    unittest.main()
