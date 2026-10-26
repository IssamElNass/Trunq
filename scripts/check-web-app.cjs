/* Browser regression checks against a running Trunq web server and disposable PostgreSQL. */
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const base = process.env.TRUNQ_WEB_URL || 'http://127.0.0.1:8080';
const db = new URL(process.env.TRUNQ_TEST_DATABASE_URL);
(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.TRUNQ_CHROMIUM ? { executablePath: process.env.TRUNQ_CHROMIUM } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 2 });
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  try {
    await page.goto(base);
    await page.getByRole('status').filter({ hasText: 'Disconnected' }).waitFor();
    assert.equal(await page.locator('#run').isDisabled(), true);
    await page.locator('#new-connection').click();
    const form = page.locator('#connection-form');
    for (const [key, value] of Object.entries({ name: 'Local PostgreSQL', host: db.hostname, port: db.port || '5432', database: db.pathname.slice(1), user: decodeURIComponent(db.username), password: decodeURIComponent(db.password) })) {
      await form.locator(`[name=${key}]`).fill(value);
    }
    await form.locator('[name=ssl_mode]').selectOption('Disable');
    await form.locator('[name=environment]').selectOption('Development');
    await page.locator('#connect-submit').click();
    await page.locator('#connection-dialog').waitFor({ state: 'hidden' });
    await page.waitForFunction(() => !document.getElementById('run').disabled);
    assert.match(await page.locator('#status').innerText(), /Connected/);
    assert.equal(await form.locator('[name=password]').inputValue(), '');
    async function run(sql) {
      await page.locator('#editor').fill(sql);
      const response = page.waitForResponse((r) => r.url().endsWith('/api/query'));
      await page.locator('#run').click(); await response;
      await page.waitForFunction(() => !document.getElementById('run').disabled);
    }
    await run("SELECT n AS id, 'row ' || n AS name, CASE WHEN n=1 THEN NULL ELSE '' END AS nullable, 9223372036854775807::bigint AS exact_value FROM generate_series(1,1201) n");
    assert.match(await page.locator('#result-summary').innerText(), /500 rows loaded/);
    assert.equal(await page.locator('td.null').first().innerText(), 'NULL');
    assert.match(await page.locator('#results').innerText(), /9223372036854775807/);
    await page.locator('#refresh').click();
    await page.waitForFunction(() => !document.getElementById('run').disabled);
    await page.locator('#fetch').click();
    await page.waitForFunction(() => document.getElementById('result-summary').textContent.includes(`${(1000).toLocaleString()} rows loaded`));
    await page.locator('#fetch').click();
    await page.waitForFunction(() => document.getElementById('result-summary').textContent.includes(`${(1201).toLocaleString()} rows loaded`));
    assert.equal(await page.locator('#fetch').isVisible(), false);
    assert.ok(await page.locator('tbody tr').count() <= 102, 'Grid should render a bounded row window');
    await page.locator('#results').evaluate((node) => { node.scrollTop = node.scrollHeight; });
    await page.getByRole('cell', { name: 'row 1201', exact: true }).waitFor();
    await page.locator('#result-filter').fill('row 1201');
    assert.match(await page.locator('#result-summary').innerText(), /1 matching/);
    await page.locator('#result-filter').fill('');
    const downloaded = page.waitForEvent('download');
    await page.locator('#csv').click();
    assert.equal((await downloaded).suggestedFilename(), 'trunq-loaded-rows.csv');
    await run("SELECT '<img src=x onerror=alert(1)>' AS escaped_text");
    assert.equal(await page.locator('#results img').count(), 0);
    assert.match(await page.locator('#results').innerText(), /onerror/);
    await run('SELECT 1/0');
    assert.match(await page.locator('#results').innerText(), /division by zero/);
    await run('SELECT 42 AS answer');
    await page.locator('#theme').click();
    assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
    await page.reload();
    await page.waitForFunction(() => !document.getElementById('run').disabled);
    assert.equal(await page.locator('#editor').inputValue(), 'SELECT 42 AS answer');
    assert.equal(await page.locator('html').getAttribute('data-theme'), 'light');
    await page.locator('#theme').click();
    await run("SELECT n AS id, (ARRAY['Platform', 'Analytics', 'Operations'])[1 + n % 3] AS team, 'Connected directly from your browser' AS description, n % 2 = 0 AS active FROM generate_series(1, 30) n");
    if (process.env.TRUNQ_WEB_SCREENSHOT) await page.screenshot({ path: process.env.TRUNQ_WEB_SCREENSHOT });
    for (const width of [390, 768, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `No viewport overflow at ${width}`);
      assert.ok(await page.locator('#run').isVisible());
    }
    await page.locator('#disconnect').click();
    await page.waitForFunction(() => document.getElementById('status').textContent === 'Disconnected');
    assert.equal(await page.locator('#run').isDisabled(), true);
    assert.deepEqual(errors, []);
    console.log('Browser UI passed: connection, query, paging, virtual grid, filtering, CSV, escaping, errors, reload, themes, responsive layout.');
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
