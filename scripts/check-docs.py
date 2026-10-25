#!/usr/bin/env python3
"""Check local Markdown links, heading anchors and image paths in the checkout."""
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]

def anchors(path):
    text = path.read_text(encoding='utf-8')
    if path.suffix in ('.html', '.htm'):
        return set(re.findall(r'\bid=["\']([^"\']+)', text))
    found = set(re.findall(r'<a\s[^>]*id=["\']([^"\']+)', text))
    counts = {}
    for heading in re.findall(r'^#{1,6}\s+(.+?)\s*#*$', re.sub(r'```.*?```', '', text, flags=re.S), re.M):
        heading = re.sub(r'<[^>]+>', '', heading).lower()
        slug = re.sub(r'[^\w\- ]', '', heading).replace(' ', '-')
        n = counts.get(slug, 0)
        counts[slug] = n + 1
        found.add(slug + (f'-{n}' if n else ''))
    return found

def main():
    files = subprocess.check_output(['git', 'ls-files', '-co', '--exclude-standard', '-z', '--', '*.md'], cwd=ROOT).decode().split('\0')
    errors = []
    checked = 0
    cache = {}
    for name in sorted(set(filter(None, files))):
        source = ROOT / name
        if not source.is_file():
            continue
        text = re.sub(r'```.*?```', '', source.read_text(encoding='utf-8'), flags=re.S)
        targets = re.findall(r'\]\(([^\s)]+)(?:\s+"[^"]*")?\)', text)
        targets += re.findall(r'<img\b[^>]*src=["\']([^"\']+)', text)
        for target in targets:
            parts = urlsplit(target.strip('<>'))
            if parts.scheme or parts.netloc:
                continue
            dest = (source.parent / unquote(parts.path)).resolve() if parts.path else source
            checked += 1
            if not dest.exists():
                errors.append(f'{name}: missing {target}')
            elif parts.fragment and dest.suffix in ('.md', '.html', '.htm'):
                if dest not in cache:
                    cache[dest] = anchors(dest)
                if unquote(parts.fragment) not in cache[dest]:
                    errors.append(f'{name}: missing heading {target}')
    print(f'Checked {checked} local links/images in {len(set(filter(None, files)))} Markdown files')
    for error in errors:
        print(error)
    return bool(errors)

if __name__ == '__main__':
    raise SystemExit(main())
