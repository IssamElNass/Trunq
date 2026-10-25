#!/usr/bin/env python3
"""Append exact, one-click platform downloads to stable and nightly notes."""

import argparse
from pathlib import Path
from urllib.parse import quote


def downloads(repository, tag, version):
    base = f"https://github.com/{repository}/releases/download/{quote(tag, safe='')}"

    def link(filename, label):
        return f"[{label}]({base}/{quote(filename, safe='')})"

    return "\n".join([
        "\n### Downloads\n",
        "| Platform | Download |",
        "|---|---|",
        f"| Windows x64 | {link(f'Trunq-{version}-windows-x86_64.exe', 'EXE (portable)')} · {link(f'Trunq-{version}-windows-x86_64.zip', 'ZIP with README and license')} |",
        f"| macOS Apple Silicon & Intel | {link(f'Trunq-{version}-macos-universal.dmg', 'DMG')} · {link(f'Trunq-{version}-macos-universal.app.zip', 'App ZIP')} |",
        f"| Debian / Ubuntu x64 | {link(f'trunq_{version}_amd64.deb', 'DEB')} |",
        f"| Other Linux x64 | {link(f'trunq-{version}-linux-x86_64.tar.gz', 'tar.gz')} |",
        "",
        f"Verify downloads with {link('SHA256SUMS.txt', 'SHA256 checksums')}. Close Trunq before replacing an existing installation.",
        "",
    ])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with args.output.open("a", encoding="utf-8") as output:
        output.write(downloads(args.repository, args.tag, args.version))
