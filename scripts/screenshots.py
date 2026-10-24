#!/usr/bin/env python3
"""Capture the real client against dev/sample.sql in a disposable database."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from urllib.parse import unquote, urlparse

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, default=root / 'target/release/trunq')
parser.add_argument('--output', type=Path, default=root / 'docs/screenshots')
args = parser.parse_args()
url = urlparse(os.environ.get('TRUNQ_TEST_DATABASE_URL', ''))
if url.scheme not in ('postgres', 'postgresql') or not url.hostname:
    parser.error('Set TRUNQ_TEST_DATABASE_URL to your disposable sample database URL')
args.output = args.output.resolve()
args.output.mkdir(parents=True, exist_ok=True)
profile = dict(id=1, name='Shop · PostgreSQL', host=url.hostname, port=url.port or 5432,
               user=unquote(url.username or 'postgres'), database=unquote(url.path.lstrip('/') or 'postgres'),
               save_password=False, only_default_db=True, environment='Local')
query = """-- name: Recent orders
SELECT o.id, c.full_name, c.country, o.status,
       o.total, o.placed_at, c.preferences
FROM shop.orders o
JOIN shop.customers c ON c.id = o.customer_id
ORDER BY o.placed_at DESC
LIMIT 100;"""
config = dict(profiles=[profile], tabs=[dict(title='Orders overview', profile_id=1,
    database=profile['database'], sql=query)], tree_open=['s1', f"d1/{profile['database']}",
    f"o1/{profile['database']}/shop", f"o1/{profile['database']}/shop.orders"])
def shot(name):
    return f"shot:{args.output / (name + '.png')}"
steps = ['resize:1440,900', 'wait:3', 'run', 'wait:2', shot('query'), 'value:1,6', 'wait:1',
    shot('json-tree'), 'close', 'open:shop.orders', 'wait:2', shot('table'), 'card', 'wait:1',
    shot('card'), 'card', 'compare', 'wait:1', shot('compare'), 'close', 'erd:shop', 'wait:2',
    shot('erd'), 'theme:light', 'wait:1', shot('erd-light'), 'theme:dark', 'structure:shop.orders',
    'wait:2', shot('structure'), 'sessions', 'wait:2', shot('sessions'), 'health', 'wait:2',
    shot('health'), 'palette:orders', 'wait:1', shot('palette'), 'close', 'tab:0',
    'resize:900,620', 'value:1,6', 'wait:1', shot('compact'), 'close', 'wait:2', 'quit']
with tempfile.TemporaryDirectory(prefix='trunq-shots-') as config_dir:
    cfg = Path(config_dir)
    (cfg / 'config.json').write_text(json.dumps(config), encoding='utf-8')
    env = dict(os.environ, TRUNQ_CONFIG_DIR=config_dir, TRUNQ_SCRIPT=';'.join(steps))
    # Keep credentials out of configuration, screenshots and shell arguments.
    if url.password:
        escape = lambda v: str(v).replace('\\', '\\\\').replace(':', '\\:')
        pgpass = cfg / 'pgpass'
        pgpass.write_text(':'.join(map(escape, [profile['host'], profile['port'], profile['database'], profile['user'], unquote(url.password)])) + '\n')
        pgpass.chmod(0o600)
        env['PGPASSFILE'] = str(pgpass)
    started_ns = time.time_ns()
    subprocess.run([str(args.binary.resolve())], env=env, check=True, timeout=300)
    assert json.loads((cfg / 'config.json').read_text()) == config, 'Script overwrote fixture workspace'
for name in ['query', 'json-tree', 'table', 'card', 'compare', 'erd', 'erd-light', 'structure', 'sessions', 'health', 'palette', 'compact']:
    path = args.output / (name + '.png')
    if not path.is_file() or path.stat().st_mtime_ns < started_ns:
        raise SystemExit(f'Missing screenshot: {path}')
    if shutil.which('cwebp'):
        target = root / 'website/assets/shots' / (name + '.webp')
        subprocess.run(['cwebp', '-quiet', '-q', '82', str(path), '-o', str(target)], check=True)
print(f'Captured 12 client screenshots in {args.output}')
