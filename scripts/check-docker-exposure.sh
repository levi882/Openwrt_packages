#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

sh -n packages/docker-guard/files/usr/bin/docker-guard
sh -n packages/docker-guard/files/usr/libexec/docker-guard-setup
sh -n packages/docker-guard/files/etc/init.d/docker-guard
sh -n packages/luci-app-docker-exposure/root/usr/libexec/rpcd/docker-exposure
node --check packages/luci-app-docker-exposure/htdocs/luci-static/resources/view/docker-exposure/overview.js
node scripts/test-docker-exposure-view.js
bash -n scripts/build-local-apks.sh
python3 scripts/test-docker-guard-setup.py

python3 - <<'PY'
import json
from pathlib import Path

root = Path('packages/luci-app-docker-exposure/root/usr/share')
menu = json.loads((root / 'luci/menu.d/luci-app-docker-exposure.json').read_text())
acl = json.loads((root / 'rpcd/acl.d/luci-app-docker-exposure.json').read_text())
entry = menu['admin/services/docker-exposure']
assert entry['action']['path'] == 'docker-exposure/overview'
assert entry['depends']['acl'] == ['luci-app-docker-exposure']
assert set(acl['luci-app-docker-exposure']['read']['ubus']['docker-exposure']) == {'list', 'status'}
assert set(acl['luci-app-docker-exposure']['write']['ubus']['docker-exposure']) == {'set_mode', 'apply'}
for base in (Path('packages/docker-guard'), Path('packages/luci-app-docker-exposure')):
    for path in base.rglob('*'):
        if path.is_file():
            assert b'\r' not in path.read_bytes(), f'CRLF in router source: {path}'
            assert not path.read_bytes().startswith(b'\xef\xbb\xbf'), f'BOM in router source: {path}'
print('PASS menu, ACL, source encoding and syntax checks')
PY
