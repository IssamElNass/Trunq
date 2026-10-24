# Trunq website

The landing page and documentation for Trunq. Plain HTML, CSS and JavaScript: no build step,
no dependencies and no third-party requests (fonts are self-hosted).

```
index.html        landing page
docs/index.html   documentation
styles.css        shared styles (light & dark theme)
site.js           shared: theme, OS detection, ⌘K search, tabs, copy buttons
home.js           landing page: trunk line, unfolding hero, million-row table, deck, console, switch, receipt, chat, dock
docs.js           docs: sidebar highlighting, "On this page", heading anchors, FAQ section
faq.js            every FAQ question (feeds the home page chat, the docs FAQ and ⌘K search)
assets/           logos, icons, screenshots (WebP, converted from docs/screenshots)
fonts/            Inter & JetBrains Mono, subset to WOFF2 (SIL OFL)
```

## Preview locally

```bash
cd website
python3 -m http.server 8000   # then open http://localhost:8000
```

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

The script uses a temporary configuration, verifies it was not overwritten, captures twelve real client views and converts them with `cwebp` when installed. To convert existing PNGs manually, run from the repository root:

```bash
for f in docs/screenshots/*.png; do cwebp -q 82 "$f" -o "website/assets/shots/$(basename "$f" .png).webp"; done
```
