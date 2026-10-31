import unittest

from perf_ab_verdict import verdict


def run(status, notes=()):
    return {"status": status, "notes": list(notes)}


class VerdictTest(unittest.TestCase):
    def test_regression_against_base_fails(self):
        base = {"scenarios": {"a": run("pass")}}
        head = {"scenarios": {"a": run("fail", ["regression: median 300 > 250"])}}
        failed, _ = verdict(base, head)
        self.assertTrue(failed)

    def test_budget_overage_shared_with_base_is_a_warning(self):
        base = {"scenarios": {"s": run("fail", ["budget: median 3500 > 3000"])}}
        head = {"scenarios": {"s": run("fail", ["budget: median 3550 > 3000"])}}
        failed, rows = verdict(base, head)
        self.assertFalse(failed)
        self.assertTrue(rows[0][1].startswith("warn"))

    def test_new_budget_overage_fails(self):
        base = {"scenarios": {"s": run("pass")}}
        head = {"scenarios": {"s": run("fail", ["budget: median 3100 > 3000"])}}
        failed, _ = verdict(base, head)
        self.assertTrue(failed)

    def test_regression_fails_even_if_base_was_over_budget(self):
        base = {"scenarios": {"s": run("fail", ["budget: median 3500 > 3000"])}}
        head = {
            "scenarios": {
                "s": run("fail", ["regression: median 4500 > 4200", "budget: median 4500 > 3000"])
            }
        }
        failed, _ = verdict(base, head)
        self.assertTrue(failed)

    def test_scenario_missing_on_base_is_judged_on_its_own(self):
        failed, _ = verdict({"scenarios": {}}, {"scenarios": {"n": run("fail", ["rss: 500 > 450"])}})
        self.assertTrue(failed)

    def test_passing_and_warning_statuses_pass(self):
        head = {"scenarios": {"a": run("pass"), "b": run("warn", ["no baseline entry"])}}
        failed, _ = verdict({"scenarios": {}}, head)
        self.assertFalse(failed)


if __name__ == "__main__":
    unittest.main()
