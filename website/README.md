# Trunq website

The landing page and documentation for Trunq. Plain HTML, CSS and JavaScript: no build step,
no dependencies and no third-party requests (fonts are self-hosted).

```
index.html        landing page
docs/index.html   documentation
styles.css        shared styles (light & dark theme)
site.js           shared: theme, OS detection, ⌘K search, tabs, copy buttons
home.js           landing page: trunk line, unfolding hero, million-row table, deck, console, switch, receipt, chat, dock
docs.js           docs: section navigation, heading anchors, reading progress, FAQ section
faq.js            every FAQ question (feeds the home page chat, the docs FAQ and ⌘K search)
assets/           logos, icons, screenshots (WebP, converted from docs/screenshots)
fonts/            Inter & JetBrains Mono, subset to WOFF2 (SIL OFL)
```

## Preview locally

```bash
cd website
python3 -m http.server 8000   # then open http://localhost:8000
```

## Check website changes

The browser regression suite uses Node.js, Playwright and Chromium, and runs in [CI](../.github/workflows/ci.yml). The website itself still has no dependencies or build step. With the local server running, execute from the repository root:

```bash
TRUNQ_CHROMIUM=/usr/bin/chromium node scripts/check-website.cjs
```

If Playwright is not available, install it outside the checkout with `npm install --prefix /tmp/trunq-web-tests playwright@1.62.1`, then prefix the command with `NODE_PATH=/tmp/trunq-web-tests/node_modules`. Set `TRUNQ_CHROMIUM` to your Chromium executable, or omit it after installing the bundled browser with `/tmp/trunq-web-tests/node_modules/.bin/playwright install chromium`. `TRUNQ_SITE_URL` changes the server address.

The checks cover mobile and desktop layouts in both themes, heading links, build tabs, exact code copying and clipboard fallback, search focus and dismissal, FAQ links, mobile navigation and unavailable browser storage. They fail on browser exceptions, missing resources, duplicate IDs or broken section links.

Keep existing heading IDs when revising the handbook so bookmarked links continue to work. Add new sections to the sidebar; headings and their text are included in search automatically. Put runnable examples in `pre[data-copyable]` and identify examples that depend on `dev/sample.sql`.

## Adding a FAQ question

Add one entry to `faq.js`. It shows up in the home page chat, the docs FAQ and ⌘K search:

```js
{ id: "ssh", k: "tunnel bastion jump host", topic: "connect", q: "Can I connect through an SSH tunnel?",
  a: "Yes. …" },
```

- `id` becomes the link anchor (`docs/#faq-ssh`), so don't change it once published.
- `topic` is one of the ids in `topics` (add a new topic there if needed).
- `k` (optional) holds extra words people might type, like synonyms or product names. They're never shown.
- In `a`, `{repo}` is the GitHub URL and `{docs:storage}` links to a docs section.

## Deploying

`.github/workflows/pages.yml` publishes this folder to GitHub Pages on every push to `main` that
touches `website/`. Enable it once under **Settings → Pages → Source: GitHub Actions**.

## Updating screenshots

Build the release client and load `dev/sample.sql` into a disposable database, then run from the repository root:

```sh
TRUNQ_TEST_DATABASE_URL=postgres://… python3 scripts/screenshots.py
```

The script uses a temporary configuration, verifies it was not overwritten, captures 21 real client views and converts them with `cwebp` when installed. To convert existing PNGs manually, run from the repository root:

```bash
for f in docs/screenshots/*.png; do cwebp -q 82 "$f" -o "website/assets/shots/$(basename "$f" .png).webp"; done
```

For the nine new extension/authentication/audit/chart views alone, add `--suite new`; a debug binary can be selected with `--binary target/debug/trunq`. Cloud connection screenshots contain example endpoints and blank secrets, not successful provider logins. Extension screenshots inspect the disposable server without changing it. Link PNGs from Markdown guides and corresponding WebP images from the handbook, with descriptive alt text and a caption explaining any example profile.

Run `python3 scripts/check-docs.py` from the repository root as well as the browser suite. The checker resolves local Markdown page/heading/image links, including hidden GitHub templates.
