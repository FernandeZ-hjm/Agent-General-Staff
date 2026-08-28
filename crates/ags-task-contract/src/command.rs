//! Structured command execution (contract v3 §7.4 verify step).
//!
//! Commands run as `program + argv` — never through a shell. The runner
//! parses a command string into argv honoring double quotes only; no shell
//! interpolation, no pipes, no redirection. Test failure never rolls back
//! source; it produces a failed receipt.

use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command};

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

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(unix)]
struct ProcessTree;

#[cfg(unix)]
impl ProcessTree {
    fn attach(_child: &mut Child) -> std::io::Result<Self> {
        Ok(Self)
    }

    fn terminate(&self, child: &mut Child) {
        let pgid = child.id() as i32;
        // SAFETY: pgid is the child's own process group (configured above);
        // a negative pid targets the entire group.
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {}
}

#[cfg(unix)]
pub fn run(spec: &CommandSpec) -> TestReceipt {
    use std::io::Read;
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
    configure_process_group(&mut command);
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
    let process_tree = match ProcessTree::attach(&mut child) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return TestReceipt {
                program: spec.program.clone(),
                argv: spec.argv.clone(),
                exit_code: -1,
                duration_ms: start.elapsed().as_millis(),
                output_digest: ags_kernel::workspace::sha256_hex(error.to_string().as_bytes()),
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
                    process_tree.terminate(&mut child);
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

#[cfg(windows)]
struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl OwnedHandle {
    fn close(mut self) -> bool {
        let handle = std::mem::replace(&mut self.0, std::ptr::null_mut());
        handle.is_null() || unsafe { windows_sys::Win32::Foundation::CloseHandle(handle) != 0 }
    }
}

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn duplicate_inheritable(file: &std::fs::File) -> std::io::Result<OwnedHandle> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let process = unsafe { GetCurrentProcess() };
    let mut duplicate = std::ptr::null_mut();
    let copied = unsafe {
        DuplicateHandle(
            process,
            file.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE,
            process,
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if copied == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(OwnedHandle(duplicate))
    }
}

#[cfg(windows)]
fn windows_quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c.is_whitespace() || c == '"') {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(ch);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn windows_command_line(spec: &CommandSpec) -> std::io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut args = Vec::with_capacity(spec.argv.len() + 1);
    args.push(windows_quote_arg(&spec.program));
    args.extend(spec.argv.iter().map(|arg| windows_quote_arg(arg)));
    let line = args.join(" ");
    if line.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "command contains a NUL byte",
        ));
    }
    Ok(std::ffi::OsStr::new(&line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect())
}

#[cfg(windows)]
fn windows_environment(spec: &CommandSpec) -> std::io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = std::env::vars_os().collect();
    for (key, value) in std::iter::once(&("AGS_COMMAND_MODE".to_string(), "structured".to_string()))
        .chain(spec.env.iter())
    {
        if key.contains(['\0', '=']) || value.contains('\0') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "environment assignment is not representable on Windows",
            ));
        }
        vars.retain(|(existing, _)| !existing.to_string_lossy().eq_ignore_ascii_case(key));
        vars.push((key.into(), value.into()));
    }
    vars.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_ascii_uppercase()
            .cmp(&right.to_string_lossy().to_ascii_uppercase())
    });
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(windows)]
fn windows_failure(
    spec: &CommandSpec,
    start: std::time::Instant,
    error: impl std::fmt::Display,
) -> TestReceipt {
    TestReceipt {
        program: spec.program.clone(),
        argv: spec.argv.clone(),
        exit_code: -1,
        duration_ms: start.elapsed().as_millis(),
        output_digest: ags_kernel::workspace::sha256_hex(error.to_string().as_bytes()),
        status: "failed".to_string(),
    }
}

#[cfg(windows)]
#[cfg(not(test))]
const WINDOWS_MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

#[cfg(windows)]
#[cfg(test)]
const WINDOWS_MAX_OUTPUT_BYTES: u64 = 64 * 1024;

#[cfg(windows)]
fn windows_output_digest(output: &mut std::fs::File) -> std::io::Result<(String, bool)> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek};

    output.seek(std::io::SeekFrom::Start(0))?;
    let mut remaining = WINDOWS_MAX_OUTPUT_BYTES;
    let mut buffer = [0_u8; 64 * 1024];
    let mut hasher = Sha256::new();
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = output.read(&mut buffer[..limit])?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    let mut probe = [0_u8; 1];
    let exceeded = output.read(&mut probe)? != 0;
    Ok((format!("{:x}", hasher.finalize()), exceeded))
}

#[cfg(windows)]
pub fn run(spec: &CommandSpec) -> TestReceipt {
    use std::os::windows::ffi::OsStrExt;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
        STARTUPINFOW,
    };

    let start = std::time::Instant::now();
    let mut output = match tempfile::tempfile() {
        Ok(file) => file,
        Err(error) => return windows_failure(spec, start, error),
    };
    let input = match tempfile::tempfile() {
        Ok(file) => file,
        Err(error) => return windows_failure(spec, start, error),
    };
    let child_output = match duplicate_inheritable(&output) {
        Ok(handle) => handle,
        Err(error) => return windows_failure(spec, start, error),
    };
    let child_input = match duplicate_inheritable(&input) {
        Ok(handle) => handle,
        Err(error) => return windows_failure(spec, start, error),
    };
    let mut command_line = match windows_command_line(spec) {
        Ok(line) => line,
        Err(error) => return windows_failure(spec, start, error),
    };
    let environment = match windows_environment(spec) {
        Ok(block) => block,
        Err(error) => return windows_failure(spec, start, error),
    };
    let cwd: Vec<u16> = spec
        .cwd
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let job_raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job_raw.is_null() {
        return windows_failure(spec, start, std::io::Error::last_os_error());
    }
    let job = OwnedHandle(job_raw);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(limits).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
    {
        return windows_failure(spec, start, std::io::Error::last_os_error());
    }

    let mut startup = STARTUPINFOW::default();
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = child_input.0;
    startup.hStdOutput = child_output.0;
    startup.hStdError = child_output.0;
    let mut info = PROCESS_INFORMATION::default();
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup,
            &mut info,
        )
    };
    drop(child_input);
    drop(child_output);
    if created == 0 {
        return windows_failure(spec, start, std::io::Error::last_os_error());
    }
    let process = OwnedHandle(info.hProcess);
    let thread = OwnedHandle(info.hThread);

    if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            let _ = TerminateProcess(process.0, 1);
            let _ = WaitForSingleObject(process.0, 5_000);
        }
        return windows_failure(spec, start, error);
    }
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        let error = std::io::Error::last_os_error();
        unsafe {
            let _ = TerminateJobObject(job.0, 1);
            let _ = WaitForSingleObject(process.0, 5_000);
        }
        return windows_failure(spec, start, error);
    }
    drop(thread);

    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms.max(1));
    let (timed_out, output_limit_hit, wait_observed_ok, mut termination_ok) = loop {
        let wait = unsafe { WaitForSingleObject(process.0, 10) };
        if wait == WAIT_OBJECT_0 {
            let exceeded = output
                .metadata()
                .map(|metadata| metadata.len() > WINDOWS_MAX_OUTPUT_BYTES)
                .unwrap_or(true);
            break (false, exceeded, true, true);
        }
        if wait != WAIT_TIMEOUT {
            break (false, false, false, false);
        }
        let exceeded = output
            .metadata()
            .map(|metadata| metadata.len() > WINDOWS_MAX_OUTPUT_BYTES)
            .unwrap_or(true);
        if exceeded {
            break (false, true, true, unsafe {
                TerminateJobObject(job.0, 1) != 0
            });
        }
        if Instant::now() >= deadline {
            break (true, false, true, unsafe {
                TerminateJobObject(job.0, 1) != 0
            });
        }
    };
    if !termination_ok {
        unsafe {
            termination_ok = TerminateJobObject(job.0, 1) != 0;
        }
    }
    let job_closed = job.close();
    let process_stopped = unsafe { WaitForSingleObject(process.0, 5_000) == WAIT_OBJECT_0 };
    let mut exit_code = u32::MAX;
    let exit_observed = unsafe { GetExitCodeProcess(process.0, &mut exit_code) != 0 };
    drop(process);

    let final_output_exceeded = output
        .metadata()
        .map(|metadata| metadata.len() > WINDOWS_MAX_OUTPUT_BYTES)
        .unwrap_or(true);
    let output_digest = windows_output_digest(&mut output);
    let digest_exceeded = output_digest
        .as_ref()
        .map(|(_, exceeded)| *exceeded)
        .unwrap_or(true);
    let output_exceeded = output_limit_hit || final_output_exceeded || digest_exceeded;
    let output_read = output_digest.is_ok();
    let exit_code = if exit_observed { exit_code as i32 } else { -1 };
    let status = if timed_out
        && wait_observed_ok
        && termination_ok
        && job_closed
        && process_stopped
        && !output_exceeded
    {
        "timeout"
    } else if !timed_out
        && termination_ok
        && wait_observed_ok
        && job_closed
        && process_stopped
        && output_read
        && !output_exceeded
        && exit_code == 0
    {
        "succeeded"
    } else {
        "failed"
    };
    TestReceipt {
        program: spec.program.clone(),
        argv: spec.argv.clone(),
        exit_code: if timed_out { -1 } else { exit_code },
        duration_ms: start.elapsed().as_millis(),
        output_digest: output_digest
            .map(|(digest, _)| digest)
            .unwrap_or_else(|error| {
                ags_kernel::workspace::sha256_hex(error.to_string().as_bytes())
            }),
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while process_is_running(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let running = process_is_running(pid);
        if running {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        assert!(!running, "grandchild survived the timeout kill");
    }

    #[cfg(windows)]
    #[test]
    fn run_enforces_timeout_windows() {
        let spec = CommandSpec {
            program: "powershell.exe".to_string(),
            argv: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ],
            cwd: std::env::temp_dir(),
            env: vec![],
            timeout_ms: 500,
        };
        let receipt = run(&spec);
        assert_eq!(receipt.status, "timeout");
        assert!(receipt.duration_ms < 10_000, "timeout must remain bounded");
    }

    #[cfg(windows)]
    #[test]
    fn timeout_kills_the_whole_process_tree_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("child.pid");
        let escaped = pidfile.display().to_string().replace('\'', "''");
        let script = format!(
            "$child = Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 30' -PassThru; Set-Content -LiteralPath '{escaped}' -Value $child.Id; Wait-Process -Id $child.Id"
        );
        let spec = CommandSpec {
            program: "powershell.exe".to_string(),
            argv: vec!["-NoProfile".to_string(), "-Command".to_string(), script],
            cwd: tmp.path().to_path_buf(),
            env: vec![],
            timeout_ms: 1_500,
        };
        let receipt = run(&spec);
        assert_eq!(receipt.status, "timeout");
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .expect("grandchild pid was written")
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while windows_process_is_running(pid) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            !windows_process_is_running(pid),
            "grandchild survived the Job Object timeout"
        );
    }

    #[cfg(windows)]
    #[test]
    fn early_exit_parent_cannot_escape_a_descendant() {
        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("child.pid");
        let escaped = pidfile.display().to_string().replace('\'', "''");
        let script = format!(
            "$child = Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 30' -PassThru; Set-Content -LiteralPath '{escaped}' -Value $child.Id"
        );
        let spec = CommandSpec {
            program: "powershell.exe".to_string(),
            argv: vec!["-NoProfile".to_string(), "-Command".to_string(), script],
            cwd: tmp.path().to_path_buf(),
            env: vec![],
            timeout_ms: 5_000,
        };
        let receipt = run(&spec);
        assert_eq!(receipt.status, "succeeded");
        let pid: u32 = std::fs::read_to_string(&pidfile)
            .expect("grandchild pid was written")
            .trim()
            .parse()
            .unwrap();
        assert!(
            !windows_process_is_running(pid),
            "descendant escaped when its parent exited early"
        );
    }

    #[cfg(windows)]
    #[test]
    fn oversized_output_fails_without_unbounded_capture() {
        let spec = CommandSpec {
            program: "powershell.exe".to_string(),
            argv: vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "$text = 'x' * 20971520; [Console]::Out.Write($text)".to_string(),
            ],
            cwd: std::env::temp_dir(),
            env: vec![],
            timeout_ms: 10_000,
        };
        let receipt = run(&spec);
        assert_eq!(receipt.status, "failed");
        assert!(
            receipt.duration_ms < 15_000,
            "oversized output blocked cleanup"
        );
    }

    #[cfg(windows)]
    #[test]
    fn final_digest_detects_bytes_written_past_the_capture_limit() {
        use std::io::Write;

        let mut output = tempfile::tempfile().unwrap();
        output
            .write_all(&vec![b'x'; WINDOWS_MAX_OUTPUT_BYTES as usize + 1])
            .unwrap();
        let (_, exceeded) = windows_output_digest(&mut output).unwrap();
        assert!(exceeded);
    }

    #[cfg(windows)]
    fn windows_process_is_running(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return false;
            }
            let mut exit_code = 0;
            let running = GetExitCodeProcess(process, &mut exit_code) != 0
                && exit_code == STILL_ACTIVE as u32;
            CloseHandle(process);
            running
        }
    }

    #[cfg(all(unix, target_os = "linux"))]
    fn process_is_running(pid: i32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(close) = stat.rfind(')') else {
            return true;
        };
        // A killed descendant may remain as a zombie until the runner's init
        // process reaps it. It is no longer executing and therefore is not a
        // surviving verify-command process.
        stat[close + 1..]
            .split_whitespace()
            .next()
            .is_some_and(|state| state != "Z")
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn process_is_running(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn unbalanced_quote_is_rejected() {
        assert!(parse_command(r#"echo "unclosed"#).is_err());
    }
}
