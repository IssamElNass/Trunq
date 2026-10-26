# Docker desktop and GitHub Container Registry

Trunq is a native GUI application. The default image runs that same binary in Xvfb with a noVNC desktop so you can use it in a browser. This adds software rendering and desktop streaming overhead; use a native release for maximum interactive performance.

## Build and run

### Pull and run the published image

Images are published at `ghcr.io/issamelnass/trunq` for Linux amd64 and arm64. Docker selects the matching architecture automatically. The first publication must complete before the commands below work; [published versions and their tags](https://github.com/IssamElNass/Trunq/pkgs/container/trunq/versions) are the source of truth for availability.

Pull the development build and start the desktop:

```sh
docker pull ghcr.io/issamelnass/trunq:dev
docker run -d --name trunq \
  -p 127.0.0.1:8080:8080 --shm-size=256m \
  -v trunq-data:/data ghcr.io/issamelnass/trunq:dev
```

With this repository's Compose file, use the published image without building locally:

```sh
TRUNQ_IMAGE=ghcr.io/issamelnass/trunq:dev docker compose up --no-build --pull always -d
```

For a stable build, substitute a published exact version or `latest`:

```sh
docker pull ghcr.io/issamelnass/trunq:latest
# Example exact version; confirm that it has been published first:
docker pull ghcr.io/issamelnass/trunq:0.1.0
```

### Build locally

```sh
docker compose up --build -d
```

Open [the local desktop](http://localhost:8080/vnc.html?autoconnect=true&resize=scale). The fixed virtual screen scales to your browser window. Configuration and history persist in the `trunq-data` volume. Connections to a PostgreSQL service in another Compose stack use its service hostname on a shared network. To connect to the host, use `host.docker.internal`, not `localhost`.

![Docker desktop querying a real PostgreSQL 18 server](screenshots/docker.png)

The desktop has no built-in authentication. The supplied Compose file publishes HTTP only on `127.0.0.1`. For remote use, keep that binding and tunnel it with SSH (`ssh -L 8080:127.0.0.1:8080 server`), or configure an authenticated HTTPS reverse proxy. Container keychain services are not configured; leave **Save in keychain** off and supply the password per session or mount a protected `.pgpass` file.

```sh
docker compose logs -f trunq
docker compose down             # keeps the saved workspace
```

File dialogs use the included Zenity fallback when no desktop portal is available. Import/export files can live in the persistent `/data` directory. Copy them between the host and desktop with Compose:

```sh
docker compose cp ./input.csv trunq:/data/input.csv
docker compose cp trunq:/data/export.csv ./export.csv
```

The container runs as UID 10001, exposes only HTTP port 8080, and stops if the client or a desktop service exits. A health check verifies the noVNC HTTP endpoint. `TRUNQ_SCREEN=1920x1080x24` changes the virtual screen. The optional `native` build target omits the browser desktop for users configuring their own X11/Wayland display and GPU access:

```sh
docker build --target native -t trunq:native .
```

## Publish manually

GitHub Actions is the normal publishing route and uses its built-in token. For a manual push, sign in to `ghcr.io` with a classic personal access token with `write:packages`. Use your own lowercase GitHub namespace, and check out the stable Git tag before publishing its version and `latest`:

```sh
docker login ghcr.io -u YOUR_GITHUB_USERNAME
docker buildx create --name trunq-builder --use  # once; or use your existing builder
docker buildx build --platform linux/amd64,linux/arm64 \
  --label org.opencontainers.image.source=https://github.com/YOUR_GITHUB_USERNAME/Trunq \
  -t ghcr.io/YOUR_LOWERCASE_USERNAME/trunq:0.1.0 \
  -t ghcr.io/YOUR_LOWERCASE_USERNAME/trunq:latest --push .
```

Emulation is required to build a foreign architecture on a single machine. The GitHub workflow configures QEMU automatically. Manual `--push` publishes immediately; run your local image and check the desktop first.

For a development image, build from `main` and use `ghcr.io/YOUR_LOWERCASE_USERNAME/trunq:dev` instead of the release tags above. GitHub documents [registry authentication and repository linking](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry).

## Publish with GitHub Actions

[The Docker workflow](../.github/workflows/docker.yml) builds and smoke-tests a desktop image on pull requests, pushes to `main`, version tags and manual runs. It verifies the HTTP endpoint and that the client process remains alive. Publishing depends on that test.

The publish job authenticates to `ghcr.io` as `github.actor` using `secrets.GITHUB_TOKEN`, with `contents: read` and `packages: write`. No additional registry credentials or repository variables are needed. The lowercase repository path determines the image name, so a fork publishes to its own namespace. The source label links the package to the repository.

New GHCR packages are private by default, even when their source repository is public. After the first push, open the [package page](https://github.com/IssamElNass/Trunq/pkgs/container/trunq), then **Package settings → Change visibility → Public** to allow anonymous pulls. If the package already exists, ensure this repository has write access under **Manage Actions access**. See [GitHub's visibility and access settings](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility). Private pulls require `docker login ghcr.io` with a classic token with `read:packages`.

Successful workflow runs for pushes to `main` publish `dev` and a `sha-<commit>` tag. To publish the current `main` without a new commit, choose **Actions → Docker → Run workflow**, select `main`, and enable **publish**. Leave **publish** disabled to run the smoke test only.

Docker image tags identify separate channels:

| Tag | Contents |
|---|---|
| `dev` | The newest successfully published development build of `main` |
| `latest` | The most recently published stable version tag |
| `0.1.0` (example) | Exact version from Git tag `v0.1.0` |
| `0.1` (example) | Major/minor alias from `v0.1.0`, updated by later `0.1.x` publications |
| `0.2.0-rc.1` (example) | Prerelease from `v0.2.0-rc.1`; does not update `latest` or a stable major/minor alias |
| `sha-<short commit>` | Every published build; normally seven commit characters |

These are tag rules, not a list of images already uploaded. Existing Git tags are not backfilled automatically. Use the [package's versions list](https://github.com/IssamElNass/Trunq/pkgs/container/trunq/versions) to see exactly which versions and aliases exist. Each successful publish also writes exact pull commands and the image digest into its Actions run summary. A bare `docker pull ghcr.io/issamelnass/trunq` means `:latest`, which will not exist until a stable tag has been published; use `:dev` for development builds.

`latest` is a mutable Docker tag. The workflow updates it when publishing a stable version tag; it is independent of GitHub's latest-release designation. Nightly desktop downloads are separate scheduled snapshots of the default branch.

Version tags such as `v0.1.0` publish version and major/minor image tags plus a commit tag. Stable version tags also update `latest`; prereleases and branch builds leave it unchanged. Manual runs follow the selected ref: `main` updates `dev`, a stable version tag updates the release tags, and other branches publish only a commit tag. Pull requests never log in or publish. Both amd64 and arm64 are built for publication; the pre-publication runtime smoke test currently runs on amd64.

Smoke and publish builds use separate cache scopes. Cache export is best-effort with a five-minute timeout, so a successfully built/uploaded image cannot turn the run red solely because cache storage fails. Compilation, desktop smoke-test, registry-login and image-upload failures still fail their jobs.

## Production access and local secrets

Keep noVNC on loopback or behind your authenticated transport. A database's least-privilege login and verified PostgreSQL TLS are still required inside the tunnel/container. Protect the persisted workspace and mounted pgpass/certificate/key files; Unix pgpass files must have no group/other access. CLI helper binaries and login environments must exist inside the container. Native SSH supports Ed25519/ECDSA and can require pre-pinned host keys. See [production guidance](production.md) for upgrade and audit boundaries.
