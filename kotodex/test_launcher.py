#!/usr/bin/env python3
"""Tests for the launcher's component supervision.

Every bug this file exists for was the same shape: which branch runs for a
component the launcher *started* versus one it *adopted*. Nothing here starts a
process — a `Child` takes its probe and its commands as arguments, so a fake
probe and a recorded `Popen` cover the whole state machine.

Run: python3 kotodex/test_launcher.py
"""
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
# `config` and `host` are imported by name from inside the launcher, so its own
# directory has to be importable before the module is loaded.
sys.path.insert(0, str(HERE))
_spec = importlib.util.spec_from_file_location("launcher", HERE / "kotodex.py")
launcher = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(launcher)


class FakeProc:
    """Stands in for `Popen`. `code` is what `poll()` answers."""

    def __init__(self, code=None):
        self.code = code
        self.terminated = False

    def poll(self):
        return self.code


class Fixture:
    """A launcher with no processes behind it."""

    def setUp(self):
        self.log_lines = []
        self.spawned = []
        self.ran = []
        self.stopped = []

        def fake_popen(cmd, **kwargs):
            self.spawned.append(cmd)
            return FakeProc()

        def fake_run(cmd, **kwargs):
            self.ran.append(cmd)
            return subprocess.CompletedProcess(cmd, 0)

        for target, name, value in (
            (launcher.subprocess, "Popen", fake_popen),
            (launcher.subprocess, "run", fake_run),
            (launcher.host, "stop_child", self.stopped.append),
            (launcher.time, "sleep", lambda _: None),
        ):
            patched = getattr(target, name)
            setattr(target, name, value)
            self.addCleanup(setattr, target, name, patched)

    def log(self, message):
        self.log_lines.append(message)

    def child(self, up=False, **kwargs):
        """A component whose binary exists — the interpreter running this."""
        state = {"up": up}
        kid = launcher.Child(
            kwargs.pop("name", "thing"),
            lambda: state["up"],
            [sys.executable],
            **kwargs,
        )
        kid.state = state
        return kid


class ChildTest(Fixture, unittest.TestCase):
    def test_one_already_running_is_adopted_not_started(self):
        kid = self.child(up=True)
        kid.ensure(self.log)
        self.assertTrue(kid.adopted)
        self.assertEqual(self.spawned, [])

    def test_one_that_is_down_is_started_and_owned(self):
        kid = self.child(up=False)
        kid.ensure(self.log)
        self.assertFalse(kid.adopted)
        self.assertEqual(self.spawned, [[sys.executable]])

    def test_a_missing_binary_is_said_rather_than_raised(self):
        kid = launcher.Child("thing", lambda: False, ["/nonexistent/binary"])
        kid.ensure(self.log)
        self.assertTrue(kid.failed)
        self.assertIn("run setup", self.log_lines[-1])

    def test_quitting_leaves_an_adopted_component_running(self):
        kid = self.child(up=True)
        kid.ensure(self.log)
        kid.stop(self.log)
        self.assertEqual(self.stopped, [])

    def test_hiding_stops_an_adopted_component_because_it_was_asked_for(self):
        kid = self.child(up=True, stop_cmd=["stop-me"])
        kid.ensure(self.log)
        kid.stop(self.log, force=True)
        self.assertEqual(self.ran, [["stop-me"]])


class WatchdogTest(Fixture, unittest.TestCase):
    def running_child(self, **kwargs):
        kid = self.child(up=False, **kwargs)
        kid.ensure(self.log)
        return kid

    def test_a_live_child_is_left_alone(self):
        kid = self.running_child()
        self.assertFalse(kid.check(self.log))
        self.assertEqual(len(self.spawned), 1)

    def test_a_clean_exit_stops_the_launcher(self):
        kid = self.running_child()
        kid.proc.code = 0
        self.assertTrue(kid.check(self.log))

    def test_a_crash_is_restarted(self):
        kid = self.running_child()
        kid.proc.code = 1
        self.assertFalse(kid.check(self.log))
        self.assertEqual(len(self.spawned), 2)

    def test_a_component_that_keeps_crashing_gives_up_and_says_which(self):
        kid = self.running_child()
        for _ in range(launcher.MAX_RESTARTS + 1):
            kid.proc.code = 1
            kid.check(self.log)
        self.assertTrue(kid.failed)
        self.assertIn("giving up", self.log_lines[-1])

    def test_an_adopted_component_is_never_restarted(self):
        kid = self.child(up=True)
        kid.ensure(self.log)
        kid.proc = FakeProc(code=1)
        self.assertFalse(kid.check(self.log))
        self.assertEqual(self.spawned, [])

    def test_an_unsupervised_component_being_gone_is_a_choice(self):
        kid = self.running_child(supervised=False)
        kid.proc.code = 1
        self.assertFalse(kid.check(self.log))
        self.assertEqual(len(self.spawned), 1)


class RestartTest(Fixture, unittest.TestCase):
    def restart(self, kids):
        guard = {"until": 0.0}
        launcher.restart_running(kids, self.log, guard)
        return guard

    def test_one_we_started_is_stopped_and_comes_back_as_ours(self):
        kid = self.child(up=False)
        kid.ensure(self.log)
        self.restart(kids=[kid])
        self.assertEqual(self.stopped, [kid])
        self.assertFalse(kid.adopted)
        self.assertEqual(len(self.spawned), 2)

    def test_an_adopted_bare_binary_is_taken_over(self):
        taken = []
        kid = self.child(
            up=True, restart_cmd=["restart-me"], stop_adopted=lambda: taken.append(True)
        )
        kid.ensure(self.log)
        kid.state["up"] = False
        self.restart(kids=[kid])
        self.assertEqual(taken, [True])
        self.assertEqual(self.ran, [])
        self.assertFalse(kid.adopted)

    def test_someone_elses_component_is_asked_rather_than_stopped(self):
        kid = self.child(up=True, restart_cmd=["restart-me"])
        kid.ensure(self.log)
        self.restart(kids=[kid])
        self.assertEqual(self.ran, [["restart-me"]])
        self.assertEqual(self.stopped, [])

    def test_a_restart_waits_for_nothing(self):
        """Nothing is polled: a component this launcher owns is stopped here
        and started here, and an adopted one whose restart command detaches is
        covered by that daemon's own one-instance lock."""
        launcher.time.sleep = lambda _: self.fail("a restart slept")
        kid = self.child(up=False)
        kid.ensure(self.log)
        self.restart(kids=[kid])

    def test_the_watchdog_guard_is_cleared_at_the_end(self):
        kid = self.child(up=False)
        kid.ensure(self.log)
        guard = self.restart(kids=[kid])
        self.assertEqual(guard["until"], 0.0)

    def test_components_are_stopped_backwards_and_started_forwards(self):
        order = []
        kids = []
        for name in ("first", "second"):
            kid = self.child(up=False, name=name)
            kid.stop = lambda log, force=False, n=name: order.append(f"stop {n}")
            kid.ensure = lambda log, n=name: order.append(f"start {n}")
            kids.append(kid)
        self.restart(kids)
        self.assertEqual(
            order, ["stop second", "stop first", "start first", "start second"]
        )


if __name__ == "__main__":
    unittest.main()
