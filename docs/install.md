# Installing Trunq

[Documentation](README.md) · [User guide](usage.md) · [Troubleshooting](usage.md#troubleshooting)

## Download

Grab the latest build from the [**Releases page**](https://github.com/IssamElNass/Trunq/releases/latest):

| Platform | File |
|---|---|
| macOS 11+ (Apple Silicon and Intel) | `Trunq-<version>-macos-universal.dmg`: open it and drag **Trunq** to Applications |
| Windows 10/11 (x64) | `Trunq-<version>-windows-x86_64.zip`: unzip and run `Trunq.exe` |
| Debian / Ubuntu (x64) | `trunq_<version>_amd64.deb`: `sudo apt install ./trunq_<version>_amd64.deb` |
| Other Linux (x64) | `trunq-<version>-linux-x86_64.tar.gz`: unpack and run `./trunq` |

Every release lists SHA-256 checksums in `SHA256SUMS.txt`. Prefer to build it yourself? See [Build from source](#build-from-source).

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
