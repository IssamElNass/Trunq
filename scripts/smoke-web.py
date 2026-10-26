#!/usr/bin/env python3
"""Exercise the running browser API against a disposable PostgreSQL server."""
import concurrent.futures
import http.cookiejar
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get('TRUNQ_WEB_URL', 'http://127.0.0.1:8080').rstrip('/')
DB = urllib.parse.urlsplit(os.environ['TRUNQ_TEST_DATABASE_URL'])
CONNECTION = dict(name='Browser smoke test', host=DB.hostname, port=DB.port or 5432,
                  user=urllib.parse.unquote(DB.username or 'postgres'),
                  database=DB.path.lstrip('/') or 'postgres', ssl_mode='Disable')

class Browser:
    def __init__(self):
        self.cookies = http.cookiejar.CookieJar()
        self.http = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.cookies))
        self.csrf = self.request('session')['csrf']

    def request(self, path, data=None, status=200, token=None, extra=None):
        headers = dict(extra or {})
        if data is not None:
            headers.update({'Content-Type': 'application/json', 'X-Trunq-CSRF': self.csrf if token is None else token})
        req = urllib.request.Request(f'{BASE}/api/{path}', data=None if data is None else json.dumps(data).encode(), headers=headers)
        try:
            response = self.http.open(req, timeout=20)
        except urllib.error.HTTPError as error:
            response = error
        body = response.read()
        assert response.status == status, (path, response.status, body.decode())
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            assert status >= 400
            return {"error": body.decode()}

    def connect(self, read_only=False):
        return self.request('connect', dict(connection={**CONNECTION, 'read_only': read_only}, password=urllib.parse.unquote(DB.password or '')))

    def query(self, sql):
        return self.request('query', dict(sql=sql))


def main():
    browser = Browser()
    browser.request('query', {'sql': 'SELECT 1'}, status=403, token='invalid')
    browser.request('query', {'sql': 'SELECT 1'}, status=403, extra={'Sec-Fetch-Site': 'cross-site'})
    browser.request('session', status=403, extra={'Host': 'rebind.invalid'})
    browser.connect()
    other = Browser()
    other.request('query', {'sql': 'SELECT 1'}, status=400)
    assert browser.query("SELECT NULL::text, ''::text, 9223372036854775807::bigint")['results'][0]['rows'] == [[None, '', '9223372036854775807']]
    assert isinstance(browser.request('catalog', {})['schemas'], list)
    first = browser.query('SELECT generate_series(1, 1201) AS n')
    assert len(first['results'][0]['rows']) == 500 and first['results'][0]['more']
    # Catalog browsing uses another connection and must preserve the query portal.
    browser.request('catalog', {})
    second = browser.request('fetch', {'id': first['query_id']})
    assert len(second['rows']) == 500 and second['rows'][0] == ['501']
    last = browser.request('fetch', {'id': first['query_id']})
    assert len(last['rows']) == 201 and not last['more']
    browser.query('SELECT 2')
    browser.request('fetch', {'id': first['query_id']}, status=400)
    assert len(browser.query("SELECT ';' AS a; SELECT 2 AS b")['results']) == 2
    assert 'error' in browser.query('SELECT 1/0')['results'][0]
    assert browser.query('SELECT 42')['results'][0]['rows'] == [['42']]
    assert browser.query('BEGIN')['transaction'] == 'InTransaction'
    assert browser.query('SELECT 1/0')['transaction'] == 'Failed'
    assert browser.query('ROLLBACK')['transaction'] == 'Idle'
    assert 'SELECT' in browser.request('format', {'sql': 'select 1'})['sql']
    with concurrent.futures.ThreadPoolExecutor() as pool:
        running = pool.submit(browser.query, 'SELECT pg_sleep(10)')
        time.sleep(0.4)
        browser.request('query', {'sql': 'SELECT 1'}, status=409)
        browser.request('cancel', {})
        assert running.result(timeout=5)['results'][0]['code'] == '57014'
    assert browser.query('SELECT 1')['results'][0]['rows'] == [['1']]
    browser.request('connections', {**CONNECTION, 'password': 'must-not-persist'}, status=422)
    saved = browser.request('connections', CONNECTION)['connections']
    entry = next(c for c in saved if c['name'] == CONNECTION['name'])
    assert 'password' not in entry
    browser.request('connections/delete', {'id': entry['id']})
    browser.connect(read_only=True)
    assert browser.query('SHOW default_transaction_read_only')['results'][0]['rows'] == [['on']]
    # No permanent objects are created by this test, even with an elevated login.
    assert browser.query('CREATE TABLE public.trunq_web_readonly_test (id int)')['results'][0]['code'] == '25006'
    browser.request('disconnect', {})
    browser.request('query', {'sql': 'SELECT 1'}, status=400)
    print('Browser API smoke passed: sessions, CSRF/host checks, catalog, paging, transactions, cancellation, persistence, read-only mode.')

if __name__ == '__main__':
    main()
