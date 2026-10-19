import time
import unittest
import unittest.mock as mock
from pathlib import Path

import tools.release as release


class TestReleaseScript(unittest.TestCase):

    def test_parse_workspace_version(self):
        version = release.parse_workspace_version()
        self.assertRegex(version, r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$")

    def test_check_dep_alignment(self):
        issues = release.check_dep_alignment()
        self.assertIsInstance(issues, list)

    def test_publish_all_rate_limiting_with_fast_publishes(self):
        """When publish operations are instantaneous, publish_all sleeps for required rate limit."""
        sleep_calls = []

        def fake_sleep(seconds):
            sleep_calls.append(seconds)

        with mock.patch("tools.release.check_dep_alignment", return_value=[]), \
             mock.patch("tools.release.publish_crate", return_value=True), \
             mock.patch("time.sleep", side_effect=fake_sleep), \
             mock.patch("builtins.print"):

            ok = release.publish_all("0.2.5", dry=False)

        self.assertTrue(ok)
        # There are 33 published crates.
        # After crate 10, rate limiting kicks in for crates 11 through 33 (23 calls).
        self.assertEqual(len(sleep_calls), 23)
        for val in sleep_calls:
            self.assertAlmostEqual(val, release.RATE_LIMIT_SECONDS, delta=1.0)

    def test_publish_all_rate_limiting_with_slow_publishes(self):
        """When publish operations take longer than RATE_LIMIT_SECONDS, no sleep is needed."""
        sleep_calls = []

        def fake_sleep(seconds):
            sleep_calls.append(seconds)

        current_time = [1000.0]

        def fake_time():
            # Advance time by 10s per call (greater than RATE_LIMIT_SECONDS=8)
            t = current_time[0]
            current_time[0] += 10.0
            return t

        with mock.patch("tools.release.check_dep_alignment", return_value=[]), \
             mock.patch("tools.release.publish_crate", return_value=True), \
             mock.patch("time.time", side_effect=fake_time), \
             mock.patch("time.sleep", side_effect=fake_sleep), \
             mock.patch("builtins.print"):

            ok = release.publish_all("0.2.5", dry=False)

        self.assertTrue(ok)
        # Since each publish took 10s (> 8s rate limit), sleep should never be called.
        self.assertEqual(len(sleep_calls), 0)


if __name__ == "__main__":
    unittest.main()
