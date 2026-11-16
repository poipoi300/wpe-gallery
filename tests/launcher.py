"""Contract tests for the optional renderer launcher; never start a renderer."""
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest

path = Path(__file__).resolve().parents[1] / "integration/wallpaperengine-start"
loader = importlib.machinery.SourceFileLoader("launcher", str(path))
spec = importlib.util.spec_from_loader("launcher", loader)
launcher = importlib.util.module_from_spec(spec)
loader.exec_module(launcher)


class Launcher(unittest.TestCase):
    def setUp(self):
        self.old_env = os.environ.copy()
        self.temp = tempfile.TemporaryDirectory()
        os.environ.update(XDG_CONFIG_HOME=self.temp.name, WPE_FPS="240",
                          WPE_BG="3633967177", WPE_OUTPUT="DP-3")
        self.config = Path(self.temp.name) / "linux-wallpaperengine/assignments.json"
        self.config.parent.mkdir()

    def tearDown(self):
        os.environ.clear()
        os.environ.update(self.old_env)
        self.temp.cleanup()

    def test_legacy(self):
        args = launcher.arguments()
        self.assertEqual(args[args.index("--fps")+1], "240")
        self.assertIn("--fullscreen-pause-only-active", args)
        self.assertEqual(args[args.index("--bg")+1], "3633967177")

    def write(self, data):
        self.config.write_text(json.dumps(data))

    def test_multimonitor_deterministic_order(self):
        self.write({"version": 1, "outputs": {
            "HDMI-A-1": {"wallpaper_id": "456", "properties": {}},
            "DP-3": {"wallpaper_id": "123", "properties": {}}}})
        args = launcher.arguments()
        self.assertEqual(args[args.index("--screen-root"):], [
            "--screen-root", "DP-3", "--bg", "123", "--scaling", "fill",
            "--screen-root", "HDMI-A-1", "--bg", "456", "--scaling", "fill"])

    def test_bad_configuration_does_not_fall_back(self):
        for data in [
            {"version": 2, "outputs": {}},
            {"version": 1, "outputs": {}},
            {"version": 1, "outputs": {"DP-3": {"wallpaper_id": "--help"}}},
            {"version": 1, "outputs": {"DP-3": {"wallpaper_id": "123", "properties": {"a": 1}}}},
        ]:
            self.write(data)
            with self.assertRaises(ValueError):
                launcher.arguments()


if __name__ == "__main__":
    unittest.main()
