//! One host-adapter engine over host-specific communication protocols.
//!
//! Protocols describe how to ask a host for MCP state. This adapter owns the
//! shared execution, normalization, timeout, and evidence semantics.

use crate::{parse_mcp_list, platform_spec, McpProbeProtocol, McpProbeSpec, McpServerRegistration};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_RPC_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostProbeExecution {
    Ran { success: bool, output: String },
    Unavailable,
    TimedOut,
}

pub trait HostProbeRunner: Send + Sync {
    fn run(&self, spec: &McpProbeSpec) -> HostProbeExecution;
}

pub struct SystemHostProbeRunner;

impl HostProbeRunner for SystemHostProbeRunner {
    fn run(&self, spec: &McpProbeSpec) -> HostProbeExecution {
        match spec.protocol {
            McpProbeProtocol::DirectCommand => run_direct(spec),
            McpProbeProtocol::JsonlRpcCommand { command } => run_jsonl_rpc(spec, command),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostProbeStatus {
    Ready,
    AuthRequired,
    ConnectionFailed,
    ProtocolUnsupported,
    HostUnavailable,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostMcpReport {
    pub host: String,
    pub status: HostProbeStatus,
    pub evidence_source: String,
    pub servers: Vec<McpServerRegistration>,
    pub evidence: String,
}

impl HostMcpReport {
    pub fn find(&self, name: &str) -> Option<&McpServerRegistration> {
        self.servers
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
    }
}

pub struct HostAdapter<'a> {
    runner: &'a dyn HostProbeRunner,
}

impl<'a> HostAdapter<'a> {
    pub fn new(runner: &'a dyn HostProbeRunner) -> Self {
        Self { runner }
    }

    pub fn inspect_mcp(&self, host: &str) -> HostMcpReport {
        let Some(spec) = platform_spec(host).and_then(|platform| platform.mcp_probe) else {
            return HostMcpReport {
                host: host.to_string(),
                status: HostProbeStatus::ProtocolUnsupported,
                evidence_source: format!("host '{host}' MCP protocol"),
                servers: Vec::new(),
                evidence: "host protocol does not expose a supported read-only MCP probe"
                    .to_string(),
            };
        };

        match self.runner.run(&spec) {
            HostProbeExecution::Unavailable => HostMcpReport {
                host: host.to_string(),
                status: HostProbeStatus::HostUnavailable,
                evidence_source: spec.evidence_source.to_string(),
                servers: Vec::new(),
                evidence: format!("{} could not be started", spec.program),
            },
            HostProbeExecution::TimedOut => HostMcpReport {
                host: host.to_string(),
                status: HostProbeStatus::TimedOut,
                evidence_source: spec.evidence_source.to_string(),
                servers: Vec::new(),
                evidence: format!(
                    "{} did not answer within {} ms",
                    spec.program, spec.timeout_ms
                ),
            },
            HostProbeExecution::Ran { success, output } if success => HostMcpReport {
                host: host.to_string(),
                status: HostProbeStatus::Ready,
                evidence_source: spec.evidence_source.to_string(),
                servers: parse_mcp_list(spec.format, &output),
                evidence: output.trim().to_string(),
            },
            HostProbeExecution::Ran { output, .. } => {
                let status = if authentication_required(&output) {
                    HostProbeStatus::AuthRequired
                } else {
                    HostProbeStatus::ConnectionFailed
                };
                HostMcpReport {
                    host: host.to_string(),
                    status,
                    evidence_source: spec.evidence_source.to_string(),
                    servers: Vec::new(),
                    evidence: output.trim().to_string(),
                }
            }
        }
    }
}

pub fn inspect_host_mcp(host: &str) -> HostMcpReport {
    HostAdapter::new(&SystemHostProbeRunner).inspect_mcp(host)
}

fn authentication_required(output: &str) -> bool {
    let normalized = output.to_ascii_lowercase();
    normalized.contains("401")
        || normalized.contains("unauthorized")
        || normalized.contains("authentication required")
        || normalized.contains("auth required")
}

fn run_direct(spec: &McpProbeSpec) -> HostProbeExecution {
    match Command::new(spec.program)
        .args(spec.args)
        .envs(spec.env.iter().copied())
        .output()
    {
        Ok(output) => HostProbeExecution::Ran {
            success: output.status.success(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        },
        Err(_) => HostProbeExecution::Unavailable,
    }
}

fn run_jsonl_rpc(spec: &McpProbeSpec, command: &str) -> HostProbeExecution {
    let mut child = match Command::new(spec.program)
        .args(spec.args)
        .envs(spec.env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return HostProbeExecution::Unavailable,
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        return HostProbeExecution::Unavailable;
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        return HostProbeExecution::Unavailable;
    };

    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let request_id = "ags-host-probe";
    let request = serde_json::json!({
        "id": request_id,
        "type": "prompt",
        "message": command,
    });
    let deadline = Instant::now() + Duration::from_millis(spec.timeout_ms);
    let mut request_sent = false;
    let mut output = String::new();
    let mut verdict = None;

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(line) = receiver.recv_timeout(remaining) else {
            break;
        };
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        if !request_sent && frame.get("type").and_then(|value| value.as_str()) == Some("ready") {
            if writeln!(stdin, "{request}").is_err() || stdin.flush().is_err() {
                verdict = Some(HostProbeExecution::Unavailable);
                break;
            }
            request_sent = true;
            continue;
        }

        if frame.get("type").and_then(|value| value.as_str()) == Some("command_output") {
            if let Some(text) = frame.get("text").and_then(|value| value.as_str()) {
                if output.len().saturating_add(text.len()) > MAX_RPC_OUTPUT_BYTES {
                    verdict = Some(HostProbeExecution::Ran {
                        success: false,
                        output: "OMP RPC response exceeded the 1 MiB probe limit".to_string(),
                    });
                    break;
                }
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(text);
            }
        }

        if frame.get("id").and_then(|value| value.as_str()) == Some(request_id)
            && frame.get("type").and_then(|value| value.as_str()) == Some("response")
        {
            let success = frame
                .get("success")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            verdict = Some(HostProbeExecution::Ran { success, output });
            break;
        }
    }

    drop(stdin);
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    verdict.unwrap_or(HostProbeExecution::TimedOut)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CannedRunner(HostProbeExecution);

    impl HostProbeRunner for CannedRunner {
        fn run(&self, _spec: &McpProbeSpec) -> HostProbeExecution {
            self.0.clone()
        }
    }

    #[test]
    fn adapter_normalizes_authentication_failure() {
        let report = HostAdapter::new(&CannedRunner(HostProbeExecution::Ran {
            success: false,
            output: "HTTP 401 authentication required".to_string(),
        }))
        .inspect_mcp("omp");
        assert_eq!(report.status, HostProbeStatus::AuthRequired);
    }
}
