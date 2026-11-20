"""Exercise the actual release decision script without contacting GitHub."""
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap
import tomllib
import unittest
from unittest.mock import patch


class ReleaseGateTests(unittest.TestCase):
    def test_release_retries_missing_tags_without_replacing_existing_ones(self):
        workflow = (Path(__file__).resolve().parents[1] / ".github/workflows/release.yml").read_text(encoding="utf-8")
        script = textwrap.dedent(workflow.split("        run: |\n", 1)[1].split("\n  wheels:", 1)[0])
        sha = "a" * 40
        current = tomllib.loads((Path(__file__).resolve().parents[1] / "pyproject.toml").read_text(encoding="utf-8"))["project"]["version"]
        for event, previous, tags, expected in [
            ("pull_request", "0.0.0", "tag", "true"),
            ("pull_request", current, "", "true"),
            ("pull_request", current, "tag", "false"),
            ("workflow_dispatch", "", "", "true"),
        ]:
            with self.subTest(event=event, previous=previous, tags=tags), tempfile.TemporaryDirectory() as directory:
                output = Path(directory) / "output"

                def check_output(args, **_kwargs):
                    if args[1] == "rev-parse":
                        return sha + "\n"
                    if args[1] == "show":
                        return f'[project]\nversion = "{previous}"\n'
                    if args[1] == "ls-remote":
                        return tags
                    raise AssertionError(args)

                env = {"SOURCE_SHA": sha, "BASE_SHA": "b" * 40, "EVENT_NAME": event, "GITHUB_OUTPUT": str(output)}
                with patch.dict(os.environ, env), patch("subprocess.run"), patch("subprocess.check_output", side_effect=check_output), patch("pathlib.Path.read_text", return_value=f'[project]\nversion = "{current}"\n'):
                    exec(compile(script, "release.yml", "exec"), {})
                self.assertIn(f"should_build={expected}", output.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
