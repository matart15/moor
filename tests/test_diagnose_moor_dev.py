import importlib.util
import json
import os
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/diagnose-moor-dev.py'
spec = importlib.util.spec_from_file_location('moor_diagnostics', SCRIPT)
diagnostics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(diagnostics)


class DiagnosticsTests(unittest.TestCase):
    def test_output_excludes_credentials_urls_and_agent_identifiers(self):
        with TemporaryDirectory() as folder, patch.object(diagnostics, 'DATA', Path(folder)):
            root = Path(folder)
            token = root / 'management-token'
            token.write_text('a' * 64)
            token.chmod(0o600)
            (root / 'port').write_text('9323')
            (root / 'logs').mkdir()
            responses = {
                '/api/health': {'status': 'ok'},
                '/api/bridge': {'connection': 'Connected', 'lastError': None,
                                'observedAgents': [{'userId': 'secret-user', 'clientId': 'secret-client'}]},
                '/api/settings': {'bridge': {'enabled': True, 'paired': True,
                    'deploymentUrl': 'https://secret.example',
                    'agentProfiles': {'secret-user:secret-client': 'secret-profile'}}},
                '/api/servers': [{'id': '11111111-1111-1111-1111-111111111111',
                    'name': 'sentry', 'status': 'running', 'url': 'https://private.example'}],
            }
            with patch.object(diagnostics, 'api_get', side_effect=lambda path, token, port: responses[path]):
                result = diagnostics.collect()
            text = json.dumps(result)
            for forbidden in ('secret-user', 'secret-client', 'secret-profile', 'secret.example', 'a' * 64):
                self.assertNotIn(forbidden, text)
            self.assertEqual(result['bridge']['observedAgentsAssigned'], [True])
            self.assertEqual(result['bridge']['assignedAgentCount'], 1)
            self.assertEqual(result['servers'][0]['status'], 'running')


if __name__ == '__main__':
    unittest.main()
