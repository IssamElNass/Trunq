#!/usr/bin/env node
// Run against the local static server; see website/README.md for prerequisites.
const assert = require('node:assert/strict');
const { chromium } = require('playwright');
const base = process.env.TRUNQ_SITE_URL || 'http://127.0.0.1:8000';

(async () => {
  const browser = await chromium.launch({ executablePath: process.env.TRUNQ_CHROMIUM || undefined });
  const errors = [];
  let checks = 0;
  const pageFor = async (options = {}) => {
    const context = await browser.newContext({ reducedMotion: 'reduce', ...options });
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    page.on('response', response => { if (response.status() >= 400) errors.push(`${response.status()} ${response.url()}`); });
    return page;
  };
  const aligned = async (page, id) => {
    await page.waitForFunction(id => {
      const top = document.getElementById(id).getBoundingClientRect().top;
      const offset = parseFloat(getComputedStyle(document.documentElement).scrollPaddingTop);
      const section = document.getElementById(id).closest('section').querySelector('h2')?.id || 'introduction';
      return Math.abs(top - offset) < 3 && document.querySelector('.sidenav [aria-current]')?.getAttribute('href') === '#' + section;
    }, id);
    assert.equal(await page.locator('.sidenav [aria-current]').getAttribute('href'), `#${await page.evaluate(id => document.getElementById(id).closest('section').querySelector('h2')?.id || 'introduction', id)}`);
  };
  try {
    for (const width of [390, 1440]) for (const colorScheme of ['light', 'dark']) {
      const page = await pageFor({ viewport: { width, height: 900 }, colorScheme });
      await page.goto(`${base}/docs/`);
      await page.evaluate(() => document.fonts.ready);
      assert.equal(await page.locator('html').getAttribute('data-theme'), colorScheme);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `page overflow at ${width}px`);
      const invalid = await page.evaluate(() => {
        const ids = [...document.querySelectorAll('[id]')].map(el => el.id);
        const duplicates = ids.filter((id, i) => ids.indexOf(id) !== i);
        const broken = [...document.querySelectorAll('a[href^="#"]')].filter(a => !document.getElementById(a.hash.slice(1))).map(a => a.hash);
        return { duplicates, broken };
      });
      assert.deepEqual(invalid, { duplicates: [], broken: [] });
      await page.context().close(); checks++;
    }

    const page = await pageFor({ viewport: { width: 1440, height: 1000 } });
    await page.goto(`${base}/docs/#build`);
    await aligned(page, 'build');
    await page.locator('#tab-linux').click();
    await page.keyboard.press('End');
    assert.equal(await page.locator('#tab-win').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('Home');
    assert.equal(await page.locator('#tab-mac').getAttribute('aria-selected'), 'true');
    await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('#tab-linux').getAttribute('aria-selected'), 'true'); checks++;

    await page.evaluate(() => { Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async text => { window.copied = text; } } }); });
    await page.locator('#build-linux .copy').click();
    assert.equal(await page.evaluate(() => window.copied), await page.locator('#build-linux pre').textContent());
    await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw new Error('denied'); }; });
    await page.locator('#build-linux .copy').click();
    assert.equal(await page.evaluate(() => getSelection().toString()), await page.locator('#build-linux pre').textContent()); checks++;

    const trigger = page.locator('.docs__search');
    await trigger.click();
    await page.locator('[data-palette-input]').fill('export');
    await page.locator('.palette__item').filter({ hasText: 'Export the rows you mean' }).click();
    await aligned(page, 'export-scope');
    assert.equal(new URL(page.url()).hash, '#export-scope'); checks++;

    await trigger.click();
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('.palette__close').evaluate(el => el === document.activeElement), true);
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('[data-palette-input]').evaluate(el => el === document.activeElement), true);
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('[data-palette]').isVisible(), false);
    assert.equal(await trigger.evaluate(el => el === document.activeElement), true); checks++;

    await trigger.click();
    await page.locator('[data-palette-input]').fill('SSH');
    await page.locator('.palette__item').filter({ hasText: 'Can I connect through an SSH tunnel or bastion host?' }).click();
    assert.equal(await page.evaluate(() => document.getElementById(location.hash.slice(1)).open), true); checks++;
    await page.context().close();

    const mobile = await pageFor({ viewport: { width: 390, height: 844 }, reducedMotion: 'no-preference' });
    await mobile.goto(`${base}/docs/#editor`);
    await aligned(mobile, 'editor');
    assert.equal(await mobile.locator('[data-docs-menu]').getAttribute('open'), null);
    await mobile.locator('[data-docs-menu] > summary').click();
    await mobile.locator('.sidenav a[href="#results"]').click();
    await aligned(mobile, 'results');
    assert.equal(await mobile.locator('[data-docs-menu]').getAttribute('open'), null);
    await mobile.locator('[data-menu-btn]').click();
    await mobile.locator('[data-open-palette]').first().click();
    assert.equal(await mobile.locator('[data-menu]').isVisible(), false);
    await mobile.locator('.palette__close').click();
    assert.equal(await mobile.locator('[data-palette]').isVisible(), false); checks++;
    await mobile.context().close();

    for (const path of ['/', '/docs/']) {
      const restricted = await pageFor();
      await restricted.addInitScript(() => {
        Storage.prototype.getItem = () => { throw new DOMException('blocked', 'SecurityError'); };
        Storage.prototype.setItem = () => { throw new DOMException('blocked', 'SecurityError'); };
      });
      await restricted.goto(base + path);
      const before = await restricted.locator('html').getAttribute('data-theme');
      await restricted.locator('[data-theme-toggle]').first().click();
      assert.notEqual(await restricted.locator('html').getAttribute('data-theme'), before);
      await restricted.keyboard.press('Control+k');
      assert.equal(await restricted.locator('[data-palette]').isVisible(), true);
      await restricted.context().close(); checks++;
    }
    assert.deepEqual(errors, [], 'Browser errors or missing resources');
    console.log(`${checks} browser checks passed: layouts, anchors, tabs, clipboard, search, FAQ, mobile navigation and restricted storage.`);
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
