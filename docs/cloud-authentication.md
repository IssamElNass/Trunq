# Cloud-neutral authentication

[Documentation](README.md) · [Connections and TLS](connections.md) · [Database administration](administration.md)

Trunq separates PostgreSQL connectivity from identity acquisition. Use the same query, browsing, export and transaction tools with a local server or a managed service. Authentication does not grant network access or database privileges: configure the provider's firewall/private endpoint, identity-to-role mapping and PostgreSQL grants separately.

## Choose a method

| Identity source | Connection method | Where identity is selected |
|---|---|---|
| PostgreSQL password, SCRAM or MD5 | **Password / client certificate** | Session password, keychain or pgpass |
| PostgreSQL client certificate | **Password / client certificate**, with TLS identity files | Server certificate-auth rules; PostgreSQL User still required |
| Azure human account | **Microsoft Entra ID**: browser, device code or Azure CLI | Entra sign-in or existing CLI login |
| Azure application with client secret | **Microsoft Entra ID → Service principal** | Explicit tenant/application ID and secret environment variable |
| Azure managed identity, certificate or federated identity | **Token command → Azure CLI** | CLI login established outside Trunq |
| AWS RDS IAM, including an AWS SSO/profile session | **Token command → AWS RDS** | AWS CLI credential chain/profile |
| Google Cloud SQL IAM | **Token command → Google Cloud** | Google Cloud CLI active identity |
| Other token-as-password services or a custom broker | **Token command** | Your local executable and arguments |

This is a common interface to provider CLIs, not embedded AWS/Google SDKs. Install the chosen CLI on the machine running Trunq and sign in before connecting. Other PostgreSQL services can use the password or certificate methods without any cloud SDK. Kerberos/GSS/SSPI and PostgreSQL SASL OAUTHBEARER are not implemented; a token command only works where the server accepts a token as a PostgreSQL password.

## Token command contract

In the connection dialog choose **Token command (any provider)**, select a preset or enter an executable, and put **one argument on each line**. Arguments are passed directly, without shell splitting, quoting or expansion. An argument containing a space remains one argument; do not add shell quotes around it. `{host}`, `{port}`, `{user}` and `{database}` substitute the connection values. Braces inside those values remain literal.

The executable must print one nonempty UTF-8 token on stdout, optionally followed by a newline. Prompts, banners, JSON and multiline output are rejected. Use CLI output selectors or a small wrapper executable to produce the token. The helper inherits the application's environment and has no interactive stdin. Relative executables use the application's PATH; desktop launchers may have a different PATH from your terminal, so use an absolute executable path when needed. Windows presets use `az.cmd` and `gcloud.cmd`.

Every new physical connection obtains a fresh token, including browser sessions, new tabs, reconnects and separate file-export sessions. The command is not run on every query or fetched page. There is no Trunq token disk cache. Existing PostgreSQL sessions are not reauthenticated when a token expires. Expensive CLI startup therefore affects connection time; keep established sessions when practical.

The helper timeout defaults to 30 seconds (configurable from 1 to 300). Output is limited to 64 KiB. On failure Trunq suppresses stdout/stderr and argument contents in the error; diagnose the same command in a terminal without posting its token. A timeout kills the direct helper process. Wrappers should not leave detached subprocesses behind.

Only run helpers you trust: saved profiles contain executable paths and arguments. **Arguments are ordinary workspace configuration, not secret storage.** Keep client secrets, tokens and shell commands out of them; use your CLI's credential store, an inherited environment variable or a broker. Trunq does not invoke a shell unless you explicitly configure a shell executable.

Token-command profiles default to `verify-full` and enforce at least `require` over TCP. Use **Advanced → Root CA bundle** for provider/private PEM CAs. `require` encrypts but does not verify the server identity. For a local proxy that itself performs automatic IAM authentication and does not accept PostgreSQL TLS locally, use the ordinary password method with the proxy's documented local connection settings; the proxy owns the remote TLS and token lifecycle.

## Azure options

### Human sign-in and account changes

Browser/device-code sign-in supports the global, China and US Government clouds. Interactive cached credentials are separated by flow, cloud, tenant, application ID and the configured PostgreSQL User. Different database group-role names can consequently prompt separately. After this update an older interactive refresh-token entry is not reused automatically; another sign-in may be required.

The native **Azure CLI** flow asks the CLI for a token at each new connection, so changing the selected CLI identity is respected. The CLI owns that login; **Sign out** in Trunq clears Trunq's matching cache/keychain entry but does not run `az logout` or close database sessions. A database group role does not necessarily equal the token's user name. Set User explicitly for group/application identities. [Microsoft's PostgreSQL Entra setup](https://learn.microsoft.com/en-us/azure/postgresql/security/security-entra-configure) describes server-side principal provisioning.

### Service principal with a secret

1. Launch Trunq with an environment variable containing the application secret. Desktop applications launched from a dock/start menu may not inherit terminal variables.
2. Choose **Microsoft Entra ID → Service principal (secret from environment)**.
3. Supply the correct cloud, explicit tenant and application client ID.
4. Enter the **variable name** in Secret variable; empty selects `AZURE_CLIENT_SECRET`. Never paste the secret value into this field.
5. Enter the PostgreSQL role that your Entra administrator mapped to that principal and test the connection with `verify-full`.

Only the variable's name is saved. Tokens remain in memory and are reacquired near expiry; this flow stores no refresh token. Rotate the secret in the launch environment and restart Trunq to guarantee use of the new value. Client credentials use the PostgreSQL resource's `/.default` scope. [Microsoft's client-credentials protocol](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow) also documents certificate and federated credentials.

### Managed identity, certificate or federation through CLI

Prepare the intended identity in your terminal using the supported Azure CLI login for that environment. For an Azure compute resource with a managed identity, this can be `az login --identity`; user-assigned identities need the CLI's explicit identity selector. This is not a managed identity for an arbitrary local laptop.

For a service principal using a certificate, or a workload with a federated assertion, use the CLI's corresponding `az login --service-principal` options outside Trunq. Then choose the **Azure CLI** native flow or the **Azure CLI** token-command preset. The latter uses `--resource-type oss-rdbms` and the CLI's selected cloud. Verify the active account/cloud before connecting. Trunq does not silently perform a new CLI login. See [Azure CLI login options](https://learn.microsoft.com/en-us/cli/azure/reference-index#az-login).

## AWS RDS IAM

Enable IAM database authentication, provision the PostgreSQL user and IAM permissions, and establish your CLI credentials (for example an AWS SSO/profile login). In Trunq choose the AWS RDS preset, set the **real RDS endpoint**, port and database user, and replace the preset region `us-east-1` with the instance's region. To choose a named profile, append two argument lines: `--profile` and the profile name. Set `verify-full` and the current RDS CA bundle.

Do not replace the hostname with an arbitrary DNS alias or localhost when generating the token: the signature is endpoint-specific. Through Trunq's SSH tunnel keep the real database endpoint in Host; the bastion resolves it. Tokens normally last 15 minutes, and temporary AWS credentials must still be valid when connecting. A token's expiry does not end an established database session. Follow the [RDS IAM requirements and limits](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.html) and [connection instructions](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.Connecting.html).

## Google Cloud SQL IAM

Enable IAM database authentication, add the IAM database user and required IAM roles, grant PostgreSQL object access, and sign in to gcloud. The preset runs `gcloud sql generate-login-token`; select the desired CLI account/configuration or append supported account/impersonation arguments. Use the provider's exact database username: user and service-account naming differ. For a direct connection, configure network reachability and the instance's TLS/CA/client-certificate requirements.

Alternatively run Cloud SQL Auth Proxy v2 externally with `--auto-iam-authn` using the intended ADC identity, then connect Trunq to that local listener using the ordinary password method and the proxy's documented local settings. Trunq does not start or supervise the proxy. The CLI's login credentials and the proxy's ADC credentials are separate selectors; check both. [Google's IAM login guide](https://docs.cloud.google.com/sql/docs/postgres/iam-logins) covers direct/manual versus proxy-managed authentication.

## Client certificate authentication

Under **Advanced**, provide absolute paths to:

- Root CA bundle: one or more PEM certificates, added to the platform's trust roots.
- Client certificate: PEM certificate chain, leaf first, if the server requires it.
- Client key: matching, **unencrypted PKCS #8 PEM** private key.

Supply both client certificate and key; encrypted keys and PKCS #12 archives are not accepted by these fields. Protect the key file with filesystem permissions. Certificate paths are saved in the workspace, but contents are not copied into it or the keychain. Recheck paths after moving a workspace. The server may authenticate using the certificate alone or additionally request a password; Trunq supports both. Choose `verify-full`; installing a client identity does not itself verify the server.

## Troubleshooting

| Symptom | Check |
|---|---|
| Helper cannot start | Absolute executable path, installation and desktop PATH |
| Helper timed out | Sign in in a terminal first; disable prompts; inspect wrapper subprocesses |
| Helper output rejected | Use raw token output; remove JSON, banners and diagnostics from stdout |
| Token accepted by CLI but database login denied | Resource/audience, cloud, tenant, endpoint, role mapping, IAM grants and token age |
| Azure application cannot infer a username | Set the provisioned PostgreSQL User explicitly |
| AWS token fails through a tunnel | Sign with the original RDS endpoint and correct region/user |
| Certificate rejected | PEM/PKCS #8 formats, matching key, hostname/SAN, current CA and file permissions |
| One tab works while another fails | The new connection acquired fresh credentials; old sessions retain their original identity |

Live cloud-provider logins require the corresponding infrastructure. Local helper, TLS and SQL tests are not evidence that your tenant's policies or cloud IAM grants are correct.
