#!/usr/bin/env python3
"""Resolve one source commit and package identity for stable/nightly builds."""

import argparse
from datetime import datetime, timezone
import re
import subprocess
import tomllib
from pathlib import Path


def metadata(version, sha, event, requested_tag, run_number, now):
    match = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version)
    if not match or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("invalid package version or source commit")
    nightly = event in {"schedule", "pull_request"} or requested_tag == "nightly"
    if nightly:
        if not re.fullmatch(r"[1-9][0-9]*", run_number):
            raise ValueError("nightly builds need a positive workflow run number")
        date = now.astimezone(timezone.utc).strftime("%Y%m%d")
        identity = f"nightly-{date}-{run_number}-g{sha[:7]}"
        return {
            "tag": identity,
            "version": identity,
            "source_version": version,
            "sha": sha,
            "nightly": "true",
            "prerelease": "true",
            "deb_version": f"{match[1]}~nightly.{date}.{run_number}+g{sha[:7]}",
        }
    expected = f"v{version}"
    if requested_tag != expected:
        raise ValueError(f"tag {requested_tag!r} does not match Cargo.toml version {expected}")
    return {
        "tag": expected,
        "version": version,
        "source_version": version,
        "sha": sha,
        "nightly": "false",
        "prerelease": str("-" in version.split("+", 1)[0]).lower(),
        "deb_version": "",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--event", required=True)
    parser.add_argument("--tag", default="")
    parser.add_argument("--run-number", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    values = metadata(version, sha, args.event, args.tag, args.run_number, datetime.now(timezone.utc))
    with args.output.open("a") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
