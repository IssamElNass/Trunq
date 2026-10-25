import importlib.util
from pathlib import Path
import re
import unittest

spec = importlib.util.spec_from_file_location("downloads", Path(__file__).with_name("release-downloads.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DownloadsTests(unittest.TestCase):
    def test_links_match_complete_release_assets_in_both_channels(self):
        for version, tag in [("0.2.0", "v0.2.0"), ("nightly-20261025-42-g1234567", "nightly-20261025-42-g1234567")]:
            with self.subTest(tag=tag):
                notes = module.downloads("owner/repo", tag, version)
                links = re.findall(r"\]\((https://[^)]+)\)", notes)
                self.assertEqual(len(links), 7)
                base = f"https://github.com/owner/repo/releases/download/{tag}/"
                self.assertTrue(all(url.startswith(base) for url in links))
                self.assertEqual({url.removeprefix(base) for url in links}, {
                    f"Trunq-{version}-windows-x86_64.exe", f"Trunq-{version}-windows-x86_64.zip",
                    f"Trunq-{version}-macos-universal.dmg", f"Trunq-{version}-macos-universal.app.zip",
                    f"trunq_{version}_amd64.deb", f"trunq-{version}-linux-x86_64.tar.gz", "SHA256SUMS.txt",
                })


if __name__ == "__main__":
    unittest.main()
