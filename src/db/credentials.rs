//! Short-lived credentials from a local provider. No shell, disk token cache or
//! implicit login: the user's CLI/helper owns identity selection and renewal.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

const MAX_OUTPUT: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TokenCommand {
    pub executable: String,
    /// One argument per line. Substitutions never undergo shell evaluation.
    pub arguments: String,
    pub timeout_s: u32,
}

impl Default for TokenCommand {
    fn default() -> Self {
        Self { executable: String::new(), arguments: String::new(), timeout_s: 30 }
    }
}

impl TokenCommand {
    pub fn azure_cli() -> Self {
        Self {
            executable: if cfg!(windows) { "az.cmd" } else { "az" }.into(),
            arguments: "account\nget-access-token\n--resource-type\noss-rdbms\n--query\naccessToken\n--output\ntsv".into(),
            ..Default::default()
        }
    }

    pub fn aws_rds() -> Self {
        Self {
            executable: "aws".into(),
            arguments: "rds\ngenerate-db-auth-token\n--hostname\n{host}\n--port\n{port}\n--username\n{user}\n--region\nus-east-1".into(),
            ..Default::default()
        }
    }

    pub fn google_cloud() -> Self {
        Self { executable: if cfg!(windows) { "gcloud.cmd" } else { "gcloud" }.into(), arguments: "sql\ngenerate-login-token".into(), ..Default::default() }
    }

    pub fn args(&self, host: &str, port: u16, user: &str, database: &str) -> Vec<String> {
        let port = port.to_string();
        self.arguments.lines().map(|line| substitute(line, &[("{host}", host), ("{port}", &port), ("{user}", user), ("{database}", database)])).collect()
    }

    /// Called on a blocking connection worker, never on the UI thread. Obtain a
    /// new token for each physical connection, including exports and reconnects.
    pub fn password(&self, host: &str, port: u16, user: &str, database: &str) -> Result<String> {
        let args = self.args(host, port, user, database);
        let output = super::session::runtime().block_on(run(&self.executable, &args, Duration::from_secs(self.timeout_s.clamp(1, 300) as u64)))?;
        parse_token(output)
    }
}

// Scan the template once so braces inside a substituted database/user name are
// literal data rather than another substitution.
fn substitute(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = template;
    while !rest.is_empty() {
        if let Some((key, value)) = values.iter().find(|(key, _)| rest.starts_with(key)) {
            out.push_str(value);
            rest = &rest[key.len()..];
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

fn parse_token(output: Vec<u8>) -> Result<String> {
    let token = String::from_utf8(output).context("credential helper output must be UTF-8")?;
    let token = token.trim();
    if token.is_empty() || token.chars().any(char::is_whitespace) || token.contains('\0') {
        bail!("credential helper must print exactly one nonempty token on stdout; disable interactive prompts and extra output");
    }
    Ok(token.to_owned())
}

/// Bounded asynchronous process execution. Kill on timeout, oversized output or
/// read failure. Never include stdout/stderr or arguments in errors: they can
/// contain tokens or credentials. Stdin is closed to prevent hidden prompts.
pub async fn run(executable: &str, args: &[String], timeout: Duration) -> Result<Vec<u8>> {
    if executable.trim().is_empty() {
        bail!("choose a credential helper executable");
    }
    let mut command = tokio::process::Command::new(executable.trim());
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().context("could not start credential helper; check its executable path and installation")?;
    let mut stdout = child.stdout.take().unwrap().take(MAX_OUTPUT + 1);
    let result = tokio::time::timeout(timeout, async {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await.context("could not read credential helper output")?;
        if output.len() > MAX_OUTPUT as usize {
            bail!("credential helper output exceeded 64 KiB");
        }
        let status = child.wait().await.context("could not wait for credential helper")?;
        if !status.success() {
            bail!("credential helper failed (exit code {:?}); verify its login and configuration in your terminal", status.code());
        }
        Ok(output)
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        other => {
            let _ = child.kill().await;
            match other {
                Ok(Err(err)) => Err(err),
                _ => bail!("credential helper timed out; sign in with your provider CLI before connecting"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_spaces_shell_characters_and_literal_placeholders() {
        let command = TokenCommand { arguments: "--user\n{user}\n--endpoint={host}:{port}/{database}".into(), ..Default::default() };
        assert_eq!(command.args("db.example", 5432, "a b;$(x){host}", "db name"), ["--user", "a b;$(x){host}", "--endpoint=db.example:5432/db name"]);
    }

    #[test]
    fn rejects_empty_multiline_and_binary_tokens_without_echoing_secrets() {
        assert_eq!(parse_token(b"token\r\n".to_vec()).unwrap(), "token");
        for output in [b"".as_slice(), b"secret\nprompt", b"secret\0", b"\xffsecret"] {
            let error = format!("{:#}", parse_token(output.to_vec()).unwrap_err());
            assert!(!error.contains("secret"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn helper_is_bounded_and_does_not_echo_credentials() {
        super::super::session::runtime().block_on(async {
            let call = async |script: &str, timeout| run("/bin/sh", &["-c".into(), script.into()], timeout).await;
            assert_eq!(call("printf token", Duration::from_secs(2)).await.unwrap(), b"token");
            for script in ["printf sensitive >&2; exit 7", "exec sleep 5", "yes sensitive"] {
                let started = std::time::Instant::now();
                let error = format!("{:#}", call(script, Duration::from_millis(150)).await.unwrap_err());
                assert!(!error.contains("sensitive"));
                assert!(started.elapsed() < Duration::from_secs(2));
            }
        });
    }
}
