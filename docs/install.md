# Installing Trunq

[Documentation](README.md) · [User guide](usage.md) · [Troubleshooting](usage.md#troubleshooting)

## Download

Grab the latest build from the [**Releases page**](https://github.com/IssamElNass/Trunq/releases/latest):

| Platform | File |
|---|---|
| macOS 11+ (Apple Silicon and Intel) | `Trunq-<version>-macos-universal.dmg`: open it and drag **Trunq** to Applications |
| Windows 10/11 (x64) | `Trunq-<version>-windows-x86_64.exe`: run directly, or use the `.zip` with README and license |
| Debian / Ubuntu (x64) | `trunq_<version>_amd64.deb`: `sudo apt install ./trunq_<version>_amd64.deb` |
| Other Linux (x64) | `trunq-<version>-linux-x86_64.tar.gz`: unpack and run `./trunq` |

Every release lists SHA-256 checksums in `SHA256SUMS.txt`. Prefer to build it yourself? See [Build from source](#build-from-source).

## Nightly builds

Nightlies are development snapshots of the latest default-branch commit. Once the updated Release workflow is on the default branch, GitHub Actions builds them daily at 02:00 UTC (03:00 in Paris in winter, 04:00 in summer). GitHub can delay scheduled runs. Maintainers can also choose **Actions → Release → Run workflow** and enter `nightly` as the tag input.

Find snapshots in [nightly prereleases](https://github.com/IssamElNass/Trunq/releases?q=nightly&expanded=true). They do not replace the latest stable release or its download buttons. Every successful run creates its own tag, for example `nightly-20261024-17-gabcdef0`, containing the UTC date, workflow run number and source commit prefix.

Each release has a **Downloads** table with direct links to the portable Windows `.exe`, Windows `.zip`, universal macOS `.dmg` / `.app.zip`, and Linux `.deb` / `.tar.gz`. These are release assets; downloading does not require opening an Actions artifact. Checksums cover the standalone `.exe` too. Updates use these downloads: close Trunq, replace the application and reopen it. There is no automatic in-app updater.

| Platform | Nightly asset |
|---|---|
| macOS, Apple Silicon and Intel | `Trunq-nightly-<identity>-macos-universal.dmg` or `.app.zip` |
| Debian / Ubuntu, x86_64 | `trunq_nightly-<identity>_amd64.deb` |
| Other Linux, x86_64 | `trunq-nightly-<identity>-linux-x86_64.tar.gz` |
| Windows, x86_64 | `Trunq-nightly-<identity>-windows-x86_64.zip` |
| Build identity and integrity | `BUILD_INFO.txt` and `SHA256SUMS.txt` |

`<identity>` is the date, run number and commit prefix from that release. The app may still display the Cargo package version; keep `BUILD_INFO.txt` or the asset filename when reporting a nightly issue. Debian packages receive a dated nightly package version that sorts before the matching stable version.

All three platform builds and their snapshot unit tests must succeed before publication. A failed platform build does not publish a new release. Nightlies may contain behaviour changes and are not promoted to stable automatically. Back up your workspace before trying one; use [a separate workspace](settings.md#start-an-isolated-workspace) if you want to keep its settings separate. On macOS, copying another app named Trunq into Applications can replace the existing copy.

To return to stable, close Trunq and install a stable release. Restore a workspace backup if its settings need reverting. Installing an older Debian package may require an explicit version selection or downgrade through the package manager.

## Before upgrading a production workstation

Back up the workspace and review [credential/key compatibility changes](production.md#upgrade-and-credential-changes). Older saved database/SSH secrets require re-entry, and RSA-only SSH configurations need Ed25519/ECDSA. A nightly preview does not replace staging verification or the stable release.

## Verify a download

Download `SHA256SUMS.txt` from the same release as the asset. Compute that asset's SHA-256 and compare it with the matching filename in the manifest. For example, after substituting the downloaded filename:

```sh
# Linux
sha256sum ./downloaded-trunq-package.tar.gz
# macOS
shasum -a 256 ./downloaded-Trunq.dmg
```

```powershell
# Windows PowerShell
Get-FileHash -LiteralPath .\downloaded-Trunq.zip -Algorithm SHA256
```

A checksum mismatch means the file does not match that release's manifest; download it again before installing. Checksums identify the bytes, while `BUILD_INFO.txt` identifies a nightly's source commit and workflow run.

## Build from source

The easiest way is a [pre-built download](#download). Building requires **Rust 1.95+**, matching the GUI dependencies. To build it yourself:

### macOS

```bash
# 1. Install Rust (once)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Get the code and build Trunq.app
git clone https://github.com/IssamElNass/Trunq.git trunq
cd trunq
./scripts/bundle-macos.sh --install      # builds and copies Trunq.app to /Applications
```

Then open **Trunq** from Launchpad or Spotlight like any other app.

```bash
./scripts/bundle-macos.sh          # only build target/release/bundle/Trunq.app
./scripts/bundle-macos.sh --dmg    # also create target/release/bundle/Trunq.dmg
```

> The app is ad-hoc signed. If macOS says it "can't be opened because it is from an unidentified developer",
> right-click **Trunq.app** and choose **Open** once, or run `xattr -dr com.apple.quarantine /Applications/Trunq.app`.

Requirements: macOS 11 or newer (Intel or Apple Silicon) and the Xcode command line tools (`xcode-select --install`).

### Linux

```bash
# Debian / Ubuntu build dependencies
sudo apt install build-essential pkg-config libssl-dev libxkbcommon-dev libwayland-dev
# Fedora: sudo dnf install gcc pkgconf openssl-devel libxkbcommon-devel wayland-devel

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/IssamElNass/Trunq.git trunq && cd trunq
cargo build --release
sudo install -m 755 target/release/trunq /usr/local/bin/trunq
trunq
```

Passwords are stored in the Secret Service (GNOME Keyring or KWallet).

### Windows

1. Install [Rust](https://rustup.rs) and choose the MSVC toolchain. Install the **Desktop development with C++** workload from [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and a Windows SDK if they are not already installed.
2. In a terminal:

```powershell
git clone https://github.com/IssamElNass/Trunq.git trunq
cd trunq
cargo build --release
.\target\release\trunq.exe
```

Passwords are stored in the Windows Credential Manager.

### Run from source

```bash
cargo run              # debug build, fastest to compile
cargo run --release    # optimised build, what you want for real use
cargo test             # run the test suite
```

The first build downloads and compiles the dependencies and takes a few minutes. Later builds are incremental.
