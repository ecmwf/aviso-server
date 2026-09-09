"""Local launcher contracts, without Docker or network access."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent


class LauncherTests(unittest.TestCase):
    def test_nats_config_is_private_and_does_not_print_token(self):
        with tempfile.TemporaryDirectory() as directory:
            env = dict(os.environ, CONFIG_DIR=directory, SKIP_DOCKER="1",
                       ENABLE_AUTH="true", TOKEN="synthetic-contract-token")
            result = subprocess.run(["bash", str(SCRIPTS / "init_nats.sh")],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn(env["TOKEN"], result.stdout + result.stderr)
            config = Path(directory) / "nats-server.conf"
            self.assertEqual(config.stat().st_mode & 0o777, 0o600)
            self.assertIn('token: "synthetic-contract-token"', config.read_text())
            env["TOKEN"] = 'invalid"token'
            result = subprocess.run(["bash", str(SCRIPTS / "init_nats.sh")],
                                    env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('token: "synthetic-contract-token"', config.read_text())

    def test_auth_bind_arguments_and_invalid_input_preserves_container(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            docker = root / "docker"
            docker.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "with open(os.environ['DOCKER_LOG'], 'a') as log:\n"
                "    log.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                "if sys.argv[1] == 'ps': print('contract-auth')\n"
            )
            docker.chmod(0o755)
            log = root / "calls"
            for address, port, expected in [
                ("127.0.0.1", "9090", "127.0.0.1:9090:8080"),
                ("::1", "9090", "[::1]:9090:8080"),
                ("[::]", "9090", "[::]:9090:8080"),
                ("[::1", "9090", None),
                ("fe80::1%eth0", "9090", None),
                ("[fe80::1%eth0]", "9090", None),
                ("localhost", "9090", None),
                ("127.0.0.1", "65536", None),
                ("127.0.0.1", "0", None),
            ]:
                with self.subTest(address=address, port=port):
                    log.write_text("")
                    env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}",
                               DOCKER_LOG=str(log), AUTH_O_TRON_CONTAINER_NAME="contract-auth",
                               AUTH_O_TRON_BIND_ADDRESS=address, AUTH_O_TRON_PORT=port,
                               AUTH_O_TRON_CONFIG_FILE=str(SCRIPTS / "example_auth_config.yaml"))
                    result = subprocess.run(["bash", str(SCRIPTS / "auth-o-tron-docker.sh"),
                                             "start", "--detach"], env=env, capture_output=True)
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    if expected:
                        self.assertEqual(result.returncode, 0, result.stderr)
                        run = next(call for call in calls if call[0] == "run")
                        self.assertEqual(run[run.index("-p") + 1], expected)
                        self.assertIn(["rm", "-f", "contract-auth"], calls)
                    else:
                        self.assertNotEqual(result.returncode, 0)
                        self.assertFalse(any(call[0] in ("rm", "run", "pull") for call in calls))


if __name__ == "__main__":
    unittest.main()
