#!/usr/bin/env python3
"""Exercise installation/startup against fake UCI, Docker and crontab only."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap

MOCK = r'''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
p = Path(os.environ['DOCKER_GUARD_TEST_STATE'])
s = json.loads(p.read_text())
command = Path(sys.argv[0]).name
args = [a for a in sys.argv[1:] if a != '-q']
def save(): p.write_text(json.dumps(s))
if command == 'uci':
    if args[0] == 'show':
        print('docker_exposure.settings=settings')
        for sec, values in s['containers'].items():
            print(f'docker_exposure.{sec}=container')
            for key, value in values.items(): print(f"docker_exposure.{sec}.{key}='{value}'")
    elif args[0] == 'get':
        if args[1] == 'docker_exposure.settings.default_mode': print(s['default_mode'])
        elif args[1] == 'docker_exposure.settings.exempt_ports':
            if 'exempt_ports' not in s: sys.exit(1)
            print(s['exempt_ports'])
        else:
            parts = args[1].split('.')
            value = s['containers'].get(parts[1], {}).get(parts[2])
            if value is None: sys.exit(1)
            print(value)
    elif args[0] == 'add':
        sec = 'cfg' + str(s['next_id']); s['next_id'] += 1
        s['containers'][sec] = {}; save(); print(sec)
    elif args[0] == 'set':
        key, value = args[1].split('=', 1)
        _, sec, field = key.split('.')
        s['containers'][sec][field] = value; save()
    elif args[0] == 'commit': s['commits'] += 1; save()
    else: sys.exit(2)
elif command == 'docker':
    if s['docker_unavailable']: sys.exit(1)
    print('\n'.join(s['names']))
elif command == 'crontab':
    if args == ['-l']: print(s['cron'], end='')
    else: s['cron'] = Path(args[0]).read_text(); save()
elif command == 'logger': s['messages'].append(' '.join(args)); save()
elif command == 'ip': print('default via 192.0.2.1 dev wan')
elif command == 'nft': print('table inet fw4 {}')
elif command == 'ubus': print('{}')
else: sys.exit(2)
'''


def main():
    setup = Path(__file__).resolve().parent.parent / 'packages/docker-guard/files/usr/libexec/docker-guard-setup'
    with tempfile.TemporaryDirectory(prefix='docker-guard-setup-test-') as temp:
        root = Path(temp)
        mock = root / 'mock'
        mock.write_text(textwrap.dedent(MOCK))
        mock.chmod(0o755)
        for name in ('uci', 'docker', 'crontab', 'logger', 'ip', 'nft', 'ubus'):
            (root / name).symlink_to(mock)
        state = root / 'state.json'
        data = {
            'default_mode': 'lan', 'next_id': 1, 'commits': 0,
            'containers': {'existing': {'name': 'already-there', 'mode': 'wan'}},
            'names': ['already-there', 'new.name-with-dash'], 'docker_unavailable': False,
            'cron': '# unrelated\n0 3 * * * /usr/bin/backup\n* * * * * docker-guard check\n* * * * * /usr/bin/docker-guard check # duplicate\n',
            'messages': [],
        }
        state.write_text(json.dumps(data))
        env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ['PATH'], DOCKER_GUARD_TEST_STATE=str(state))

        def run(*args):
            subprocess.run(['/bin/sh', str(setup), *args], env=env, check=True)
            return json.loads(state.read_text())

        first = run()
        assert first['containers']['existing']['mode'] == 'wan'
        assert any(c == {'name': 'new.name-with-dash', 'mode': 'lan'} for c in first['containers'].values())
        assert first['cron'].count('docker-guard check') == 1
        assert '# unrelated\n0 3 * * * /usr/bin/backup\n' in first['cron']
        print('PASS startup retains existing policy, registers current containers, preserves unrelated cron')

        again = run()
        assert again['containers'] == first['containers']
        assert again['commits'] == first['commits']
        assert again['cron'] == first['cron']
        print('PASS repeated startup is idempotent')

        removed = run('--remove-cron')
        assert removed['containers'] == first['containers']
        assert removed['cron'] == '# unrelated\n0 3 * * * /usr/bin/backup\n'
        print('PASS removal only removes guard cron job')

        removed['docker_unavailable'] = True
        state.write_text(json.dumps(removed))
        unavailable = run()
        assert unavailable['containers'] == first['containers']
        assert unavailable['commits'] == first['commits']
        assert unavailable['messages']
        assert unavailable['cron'].count('docker-guard check') == 1
        print('PASS unavailable Docker preserves configuration and reports skipped discovery')

        unavailable.update(docker_unavailable=False, default_mode='wan', names=['later-container'])
        state.write_text(json.dumps(unavailable))
        configured = run()
        assert any(c == {'name': 'later-container', 'mode': 'wan'} for c in configured['containers'].values())
        print('PASS startup registration honors default_mode')

        guard = setup.parent.parent / 'bin/docker-guard'
        status = subprocess.check_output(['/bin/sh', str(guard), 'status'], env=env, text=True)
        assert 'whitelist=\n' in status
        configured['exempt_ports'] = '443 54321'
        state.write_text(json.dumps(configured))
        status = subprocess.check_output(['/bin/sh', str(guard), 'status'], env=env, text=True)
        assert 'whitelist=443 54321\n' in status
        print('PASS no bundled port exceptions; local UCI exceptions are respected')


if __name__ == '__main__':
    main()
