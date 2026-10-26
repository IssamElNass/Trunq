# Docker browser client and Docker Hub

The default Docker image serves a browser-rendered interface at [localhost:8080](http://localhost:8080/). Text, controls and results render at your browser's resolution and zoom level. No VNC, virtual display or desktop streaming is involved.

## Build and run

```sh
docker compose up --build -d
```

Open [Trunq](http://localhost:8080/), click **+ Connection**, enter your PostgreSQL details and press **Connect**. Use `host.docker.internal` for PostgreSQL on your host. For another container, join a shared Docker network and use the database service hostname. `localhost` inside the container refers to the container itself.

The supplied Compose file publishes HTTP only on `127.0.0.1` and persists saved connection details in the `trunq-data` volume. When upgrading from the old desktop image, rebuild and open `/`; the old `/vnc.html` address is no longer used. Existing native workspace files remain in the volume, but the browser client has its own `web-connections.json`; add your connections in the browser once.

![Browser-rendered Trunq querying PostgreSQL](screenshots/docker.png)

```sh
docker compose logs -f trunq
docker compose down             # keeps the saved connections
```

## Browser workflow and current scope

- Save, edit and delete named connections; choose environment, startup read-only mode and PostgreSQL TLS mode. PEM CA/client certificate/key paths refer to files mounted inside the container.
- Browse and filter schemas, tables and views. Click a table to open a SELECT query. Catalog refresh uses a separate connection and preserves the query cursor.
- Open multiple SQL editor tabs, format SQL, run selected text or a script with **Ctrl/⌘ Enter**, cancel a running query, and inspect errors and notices. Explicit transactions stay on the same connection: run `COMMIT` or `ROLLBACK` yourself.
- Fetch results in pages of 500 rows, search loaded rows, and download **loaded rows** as CSV or JSON. JSON uses `{columns, rows}` with PostgreSQL values represented as text or null to preserve numeric precision and duplicate column names. SQL files open from and download to your browser.
- Switch light/dark themes. SQL tabs and the last 100 queries are stored in this browser's local storage; result data is kept only in page memory. Saved connection details live on the server. Passwords stay in server memory until disconnect, session expiry or shutdown; they are never saved in the web workspace.

This is a separate browser client sharing Trunq's PostgreSQL protocol and SQL code. It does not yet include native inline row editing, CSV import, full-result streaming export, autocomplete, graphical EXPLAIN/ER diagrams, grants/extension/cron screens, SSH tunnels, token helpers or Microsoft Entra sign-in. Use the native release for these features. You can run administration SQL directly in the browser editor.

Each browser session owns one query connection and one read-only catalog connection. Editor tabs share that query connection and its transaction. Starting another query replaces the previous cursor; earlier statements in a script retain up to 500 rows, and only the final statement can fetch more. Scripts accept at most 32 statements and requests up to 1 MiB. Queries start with a five-minute statement timeout and idle transactions/cursors expire after 60 seconds. Browser sessions expire after 30 minutes without API activity, releasing their connections; reconnect after expiry or container restart. At most 32 browser sessions are retained.

## Local and remote access

The browser client has no built-in user login. Keep the supplied loopback binding. For remote use, tunnel it with SSH (`ssh -L 8080:127.0.0.1:8080 server`) or configure an authenticated HTTPS reverse proxy. Saved connection metadata is shared by people with access to the service; this is a single-user workspace, not a multi-tenant deployment.

The server validates the HTTP host and browser request tokens. For a reverse proxy, set `TRUNQ_WEB_ORIGIN=https://trunq.example.com` on the container and preserve that Host header. This also enables Secure session cookies. The value must be the exact external origin, without a path. The default accepts `localhost`, `127.0.0.1` and `[::1]`; `TRUNQ_WEB_LISTEN` defaults to `0.0.0.0:8080` in Docker. Port mappings to a different local port work without changing the origin.

Use verified PostgreSQL TLS and least-privilege database roles. Mount certificates or a protected `.pgpass` file when needed; Unix password files must have no group/other access. Keychain services are not required. The runtime runs as UID 10001, exposes only HTTP port 8080, and checks `/healthz` for health. There are no X11, Wayland, GPU, VNC or screen-resolution settings in this image.

## Native target and local development

The optional `native` target still builds the desktop app for users configuring their own X11/Wayland display and GPU access:

```sh
docker build --target native -t trunq:native .
```

To run the browser backend without Docker or graphical dependencies:

```sh
cargo run --locked --no-default-features --features web
```

It listens on `127.0.0.1:8080` outside Docker. `cargo run` continues to launch the native app. A build containing both clients can use `cargo run --features web -- --web`.

## Publish manually

Replace `YOUR_NAMESPACE` with your Docker Hub account or organization. Check out the stable release tag you are publishing (for example, `v0.1.0`) before building its version tag and `latest`:

```sh
docker login
docker buildx create --name trunq-builder --use  # once; or use your existing builder
docker buildx build --platform linux/amd64,linux/arm64 \
  -t YOUR_NAMESPACE/trunq:0.1.0 -t YOUR_NAMESPACE/trunq:latest --push .
```

Emulation is required to build a foreign architecture on a single machine. The GitHub workflow configures QEMU automatically. Manual `--push` publishes immediately; run your local image and check the browser client first.

For a development image, build from `main` and use `YOUR_NAMESPACE/trunq:dev` instead of the release tags above.

## Publish with GitHub Actions

[The Docker workflow](../.github/workflows/docker.yml) builds and smoke-tests a browser image on pull requests, pushes to `main`, version tags and manual runs. It verifies health, real PostgreSQL queries, pagination, cancellation, transaction recovery, session isolation and browser interactions. Publishing depends on that test.

Set repository **Settings → Secrets and variables → Actions**:

| Type | Name | Value |
|---|---|---|
| Variable | `DOCKERHUB_USERNAME` | Docker Hub account / namespace |
| Secret | `DOCKERHUB_TOKEN` | Access token with push permission to that repository |

Successful workflow runs for pushes to `main` publish `dev` and a `sha-<commit>` tag. To publish the current `main` without a new commit, choose **Actions → Docker → Run workflow**, select `main`, and enable **publish**. Leave **publish** disabled to run the smoke test only.

Docker image tags identify separate channels:

| Tag | Contents |
|---|---|
| `latest` | The most recently published stable release |
| `0.1.0` | A specific release, built from Git tag `v0.1.0` |
| `dev` | The newest successfully published development build of `main` |
| `sha-<commit>` | An image identified by its source commit |

`latest` is a mutable Docker tag. The workflow updates it when publishing a stable version tag; it is independent of GitHub's latest-release designation. Nightly desktop downloads are separate scheduled snapshots of the default branch.

Version tags such as `v0.1.0` publish version and major/minor image tags plus a commit tag. Stable version tags also update `latest`; prereleases and branch builds leave it unchanged. Manual runs follow the selected ref: `main` updates `dev`, a stable version tag updates the release tags, and other branches publish only a commit tag. Without the namespace, the publish job is skipped. Both amd64 and arm64 are built for publication; the pre-publication runtime smoke test currently runs on amd64.

No image has been pushed as part of the local implementation. A Docker Hub namespace and token must be configured before publication.

## Production access and local secrets

Keep HTTP on loopback or behind your authenticated transport. A database's least-privilege login and verified PostgreSQL TLS are still required inside the tunnel/container. Protect the persisted workspace and mounted pgpass/certificate/key files; Unix pgpass files must have no group/other access. The native client separately supports token helpers and SSH with Ed25519/ECDSA host keys. See [production guidance](production.md) for upgrade and audit boundaries.
