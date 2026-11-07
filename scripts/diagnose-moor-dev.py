#!/usr/bin/env python3
"""Read-only, sanitized diagnostics for a locally running Moor Dev instance."""
import json
import os
from pathlib import Path
import re
import stat
import sys
from urllib import error, request

DATA = Path.home() / 'Library/Application Support/com.specdest.moor-dev'
CATEGORIES = ('error', 'failed', 'timeout', 'unauthorized', 'oauth', 'disconnected', 'reconnect')


def api_get(path, token, port):
    req = request.Request(f'http://127.0.0.1:{port}{path}', headers={'X-Moor-Token': token})
    with request.urlopen(req, timeout=5) as response:
        return json.load(response)


def collect():
    token_file = DATA / 'management-token'
    info = token_file.stat()
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
        raise ValueError('Moor token ownership or permissions invalid')
    token = token_file.read_text().strip()
    if not re.fullmatch('[0-9a-fA-F]{64}', token):
        raise ValueError('Moor token format invalid')
    port = int((DATA / 'port').read_text().strip())
    if not 1024 <= port <= 65535:
        raise ValueError('Moor port invalid')
    bridge = api_get('/api/bridge', token, port)
    settings = api_get('/api/settings', token, port)
    servers = api_get('/api/servers', token, port)
    config = settings.get('bridge') or {}
    mappings = config.get('agentProfiles') or {}
    if not isinstance(mappings, dict):
        mappings = {}
    output = {
        'health': api_get('/api/health', token, port).get('status'),
        'bridge': {
            'enabled': bool(config.get('enabled')),
            'paired': bool(config.get('paired')),
            'deploymentConfigured': bool(config.get('deploymentUrl')),
            'connection': bridge.get('connection'),
            'lastError': bridge.get('lastError'),
            'lastConnectedAtMs': bridge.get('lastConnectedAtMs'),
            'lastMessageAtMs': bridge.get('lastMessageAtMs'),
            'consecutiveFailures': bridge.get('consecutiveFailures'),
            'retryAfterSeconds': bridge.get('retryAfterSeconds'),
            'observedAgentCount': len(bridge.get('observedAgents') or []),
            'assignedAgentCount': len(mappings),
            'observedAgentsAssigned': [
                bool(isinstance(a, dict) and f"{a.get('userId', '')}:{a.get('clientId', '')}" in mappings)
                for a in (bridge.get('observedAgents') or [])[:10]
            ],
        },
        'servers': [
            {'name': s.get('name'), 'status': s.get('status')}
            for s in servers if s.get('name') in ('notion', 'sentry', 'moor_base_mcp')
        ],
    }
    # Do not output raw log lines, paths, arbitrary metadata, or URLs.
    log_counts = {}
    log_dir = DATA / 'logs'
    for s in servers:
        if s.get('name') not in ('notion', 'sentry', 'moor_base_mcp'):
            continue
        server_id = s.get('id')
        if not isinstance(server_id, str) or not re.fullmatch('[a-fA-F0-9-]{36}', server_id):
            continue
        p = log_dir / (server_id + '.log')
        counts = {k: 0 for k in CATEGORIES}
        if p.is_file() and not p.is_symlink():
            with p.open('rb') as stream:
                stream.seek(max(0, p.stat().st_size - 65536))
                lines = stream.read().decode('utf-8', 'replace').splitlines()[-200:]
            for line in lines:
                for category in CATEGORIES:
                    if category in line.lower():
                        counts[category] += 1
        log_counts[s['name']] = counts
    output['recentServerLogCategoryCounts'] = log_counts
    return output


if __name__ == '__main__':
    try:
        print(json.dumps(collect(), indent=2))
    except (OSError, ValueError, TypeError, error.URLError, json.JSONDecodeError) as exc:
        # Never print exception details that may contain local paths or credentials.
        print(json.dumps({'diagnostics': 'unavailable', 'errorType': type(exc).__name__}))
        sys.exit(1)
