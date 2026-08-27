//! Structured command execution (contract v3 §7.4 verify step).
//!
//! Commands run as `program + argv` — never through a shell. The runner
//! parses a command string into argv honoring double quotes only; no shell
//! interpolation, no pipes, no redirection. Test failure never rolls back
//! source; it produces a failed receipt.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use ags_kernel::error::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct CommandSpec {
    pub program: String,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestReceipt {
    pub program: String,
    pub argv: Vec<String>,
    pub exit_code: i32,
    pub duration_ms: u128,
    pub output_digest: String,
    pub status: String, // "succeeded" | "failed" | "timeout"
}

/// Parse a command line into argv honoring double quotes. No shell features
/// (no interpolation, pipes, redirects, globs) — anything suspicious is a
/// structured parse error. Leading `KEY=VALUE` tokens become environment
/// assignments (the only way to pass env to a structured command).
pub fn parse_command(line: &str) -> Result<CommandSpec> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            '\\' | '`' | '$' | '|' | '&' | ';' | '>' | '<' if !in_quotes => {
                return Err(Error::new(
                    "command_shell_syntax_rejected",
                    format!("shell syntax `{c}` is not allowed; provide structured argv"),
                ));
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if in_quotes {
        return Err(Error::new(
            "command_unbalanced_quote",
            "unbalanced double quote",
        ));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err(Error::new("command_empty", "empty command string"));
    }
    // Leading KEY=VALUE tokens are env assignments.
    let mut env: Vec<(String, String)> = Vec::new();
    let mut idx = 0;
    while idx + 1 < tokens.len() {
        if let Some((key, value)) = tokens[idx].split_once('=') {
            let valid = !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !key.chars().next().unwrap().is_ascii_digit();
            if valid {
                env.push((key.to_string(), value.to_string()));
                idx += 1;
                continue;
            }
        }
        break;
    }
    let program = tokens[idx].clone();
    let argv = tokens[idx + 1..].to_vec();
    Ok(CommandSpec {
        program,
        argv,
        env,
        cwd: std::env::current_dir().map_err(error_io)?,
        timeout_ms: 600_000,
    })
}

fn error_io(e: std::io::Error) -> Error {
    Error::new("cwd_resolve_failed", e.to_string())
}

pub fn run(spec: &CommandSpec) -> TestReceipt {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    use std::thread;
    use std::time::{Duration, Instant};

    let start = Instant::now();
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.argv)
        .current_dir(&spec.cwd)
        .env("AGS_COMMAND_MODE", "structured")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    // The child becomes its own process-group leader so a timeout can kill
    // the whole tree (`cargo test` spawns compiler children; killing only
    // the direct child would orphan them).
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return TestReceipt {
                program: spec.program.clone(),
                argv: spec.argv.clone(),
                exit_code: -1,
                duration_ms: start.elapsed().as_millis(),
                output_digest: ags_kernel::workspace::sha256_hex(e.to_string().as_bytes()),
                status: "failed".to_string(),
            };
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // Real timeout: poll the child; kill the whole process group when the
    // deadline passes. A hung verify command must never block the task
    // forever, and must not leave stray children behind.
    let deadline = start + Duration::from_millis(spec.timeout_ms.max(1));
    let (exit_code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code().unwrap_or(-1), false),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let pgid = child.id() as i32;
                    // SAFETY: pgid is the child's own process group (set
                    // above); negative pid targets the group.
                    unsafe {
                        libc::kill(-pgid, libc::SIGKILL);
                    }
                    let _ = child.wait();
                    break (-1, true);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break (-1, false),
        }
    };
    let mut combined = Vec::new();
    if let Some(mut out) = stdout {
        let _ = out.read_to_end(&mut combined);
    }
    if let Some(mut err) = stderr {
        let _ = err.read_to_end(&mut combined);
    }
    let duration_ms = start.elapsed().as_millis();
    let status = if timed_out {
        "timeout"
    } else if exit_code == 0 {
        "succeeded"
    } else {
        "failed"
    };
    TestReceipt {
        program: spec.program.clone(),
        argv: spec.argv.clone(),
        exit_code,
        duration_ms,
        output_digest: ags_kernel::workspace::sha256_hex(&combined),
        status: status.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quotes_and_argv() {
        let spec = parse_command(r#"cargo test --package ags-kernel -- "chain break""#).unwrap();
        assert_eq!(spec.program, "cargo");
        assert_eq!(
            spec.argv,
            vec!["test", "--package", "ags-kernel", "--", "chain break"]
        );
    }

    #[test]
    fn parse_env_assignments() {
        let spec = parse_command(r#"RUSTFLAGS="-D warnings" cargo test --workspace"#).unwrap();
        assert_eq!(spec.program, "cargo");
        assert_eq!(spec.argv, vec!["test", "--workspace"]);
        assert_eq!(
            spec.env,
            vec![("RUSTFLAGS".to_string(), "-D warnings".to_string())]
        );
    }

    #[test]
    fn shell_syntax_is_rejected() {
        assert!(parse_command("echo a && rm -rf /").is_err());
        assert!(parse_command("ls $(pwd)").is_err());
        assert!(parse_command("cat a.txt | grep x").is_err());
        assert!(parse_command("echo `whoami`").is_err());
    }

    #[test]
    fn run_enforces_timeout() {
        let spec = CommandSpec {
            program: "sleep".to_string(),
            argv: vec!["30".to_string()],
            cwd: std::env::temp_dir(),
            env: vec![],
            timeout_ms: 200,
        };
        let r = run(&spec);
        assert_eq!(r.status, "timeout");
        assert!(r.duration_ms < 10_000, "must not wait for the full sleep");
    }

    #[test]
    fn timeout_kills_the_whole_process_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("child.pid");
        let spec = CommandSpec {
            program: "sh".to_string(),
            argv: vec![
                "-c".to_string(),
                format!("sleep 30 & echo $! > '{}'; wait", pidfile.display()),
            ],
            cwd: tmp.path().to_path_buf(),
            env: vec![],
            timeout_ms: 500,
        };
        let r = run(&spec);
        assert_eq!(r.status, "timeout");
        // The grandchild (sleep) must have been killed with the group.
        let mut waited = 0;
        let pid: i32 = loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = text.trim().parse() {
                    break pid;
                }
            }
            waited += 50;
            assert!(waited < 3000, "grandchild never started");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let alive = unsafe { libc::kill(pid, 0) == 0 };
        assert!(!alive, "grandchild survived the timeout kill");
    }

    #[test]
    fn unbalanced_quote_is_rejected() {
        assert!(parse_command(r#"echo "unclosed"#).is_err());
    }
}
