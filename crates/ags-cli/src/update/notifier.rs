//! `ags update notify` — lazy, post-task, calendar-throttled update notifier.
//!
//! This is NOT a daemon, cron, or background process. It is a one-shot, on-demand
//! check meant to be called best-effort at the END of a task (Stop hook / runner /
//! MCP post-task convention). Contract:
//!
//! - **Lazy + throttled**: hits the public release/tag source at most once per
//!   [`THROTTLE_DAYS`] local calendar days. Within the window it returns
//!   `notify=false, reason=fresh` WITHOUT any network access or state write.
//! - **Silent failure**: every path exits 0. Disabled env, corrupt state, missing
//!   `git`, network/parse failure — all degrade to `notify=false` and never error.
//! - **Never auto-updates**: it only *reports*. Real updates stay behind explicit
//!   `/ags update` / `ags update apply --apply`.
//! - **Current version is always [`AGS_VERSION`]** — never hard-coded.
//! - **No credentials**: the git probe runs with `GIT_TERMINAL_PROMPT=0`; the
//!   state file holds only versions/dates, never tokens or machine secrets.

use crate::context::AGS_VERSION;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// State-file schema tag.
const STATE_SCHEMA: &str = "1.0-update-notifier";
/// Calendar-day throttle window for hitting the network.
const THROTTLE_DAYS: i64 = 7;
/// Public release/tag source. Overridable via `AGS_UPDATE_SOURCE_URL` (tests /
/// advanced operators); not a front-facing documented entry point.
const DEFAULT_SOURCE_URL: &str = "https://github.com/FernandeZ-hjm/Agent-General-Staff.git";
/// Hard wall-clock bound on the `git ls-remote` probe. MUST stay below the Stop
/// hook's 4000ms spawn timeout so the notifier always returns first.
const FETCH_TIMEOUT_MS: u64 = 3000;

// ── State file (<runtime_home>/update-state.json) ───────────────────────────

#[derive(Debug, Clone, Default)]
struct NotifierState {
    schema_version: String,
    current_version: String,
    latest_version: String,
    checked_at: u64,
    last_checked_date: String,
    last_result: String,
}

fn state_path(runtime_home: &Path) -> PathBuf {
    runtime_home.join("update-state.json")
}

/// Read prior state. Corrupt / missing → `None` (treated as "no prior state").
/// Parsed field-by-field via `serde_json::Value` so a malformed file degrades to
/// `None` instead of erroring, and so the notifier needs no `serde` derive dep.
fn read_state(runtime_home: &Path) -> Option<NotifierState> {
    let content = std::fs::read_to_string(state_path(runtime_home)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(NotifierState {
        schema_version: s("schema_version"),
        current_version: s("current_version"),
        latest_version: s("latest_version"),
        checked_at: v.get("checked_at").and_then(|x| x.as_u64()).unwrap_or(0),
        last_checked_date: s("last_checked_date"),
        last_result: s("last_result"),
    })
}

/// Best-effort state write. Failure is silent — the notifier never errors. The
/// file holds only versions/dates/result — never tokens or machine secrets.
fn write_state(runtime_home: &Path, state: &NotifierState) {
    let v = serde_json::json!({
        "schema_version": state.schema_version,
        "current_version": state.current_version,
        "latest_version": state.latest_version,
        "checked_at": state.checked_at,
        "last_checked_date": state.last_checked_date,
        "last_result": state.last_result,
    });
    if let Ok(json) = serde_json::to_string_pretty(&v) {
        let _ = std::fs::create_dir_all(runtime_home);
        let _ = std::fs::write(state_path(runtime_home), json + "\n");
    }
}

// ── Strict semver (constraint: only v?X.Y.Z; pre-release tags ignored) ──────

/// Parse a STRICT `v?X.Y.Z` version into `(major, minor, patch)`. Anything with a
/// pre-release / build suffix (`v2.8.0-beta`, `2.8.0+meta`) or a component count
/// other than three returns `None` — such tags are simply ignored when picking
/// the latest, never coerced to a release version.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim();
    let s = s
        .strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s);
    if s.contains('-') || s.contains('+') {
        return None;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

// ── Calendar-day helpers ────────────────────────────────────────────────────

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// algorithm). Used only for calendar-day differences in the throttle.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_ymd(date: &str) -> Option<(i64, i64, i64)> {
    let parts: Vec<&str> = date.trim().split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// Calendar days from `a` (YYYY-MM-DD) to `b` (YYYY-MM-DD). `None` if unparseable.
fn days_between(a: &str, b: &str) -> Option<i64> {
    let (ay, am, ad) = parse_ymd(a)?;
    let (by, bm, bd) = parse_ymd(b)?;
    Some(days_from_civil(by, bm, bd) - days_from_civil(ay, am, ad))
}

/// Today's LOCAL calendar date (YYYY-MM-DD). `AGS_UPDATE_FAKE_DATE` injects it for
/// tests; otherwise `date +%F`. `None` if unavailable (the throttle then falls
/// back to an epoch-seconds window so the notifier still does not hammer GitHub).
fn today_local() -> Option<String> {
    if let Ok(d) = std::env::var("AGS_UPDATE_FAKE_DATE") {
        let d = d.trim().to_string();
        if !d.is_empty() {
            return Some(d);
        }
    }
    let out = Command::new("date").arg("+%F").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Fetch (git ls-remote, bounded, credential-free) ─────────────────────────

/// Fetch the latest STRICT semver tag from the public source. `None` on ANY
/// failure (disabled probe, missing `git`, network, timeout, no parseable tags).
/// Test injection: `AGS_UPDATE_FAKE_FETCH_FAIL=1` forces failure;
/// `AGS_UPDATE_FAKE_LATEST=<ver>` short-circuits the real probe. Neither is a
/// front-facing documented entry point.
fn fetch_latest_version() -> Option<(u64, u64, u64)> {
    if std::env::var("AGS_UPDATE_FAKE_FETCH_FAIL")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return None;
    }
    if let Ok(v) = std::env::var("AGS_UPDATE_FAKE_LATEST") {
        return parse_version(&v);
    }
    let url =
        std::env::var("AGS_UPDATE_SOURCE_URL").unwrap_or_else(|_| DEFAULT_SOURCE_URL.to_string());
    let output = run_bounded_git_ls_remote(&url, Duration::from_millis(FETCH_TIMEOUT_MS))?;

    let mut best: Option<(u64, u64, u64)> = None;
    for line in output.lines() {
        let Some(idx) = line.find("refs/tags/") else {
            continue;
        };
        let tag = line[idx + "refs/tags/".len()..]
            .trim()
            .trim_end_matches("^{}");
        if let Some(v) = parse_version(tag) {
            if best.map(|b| v > b).unwrap_or(true) {
                best = Some(v);
            }
        }
    }
    best
}

/// Run `git ls-remote --tags <url>` bounded by a watchdog that kills the child
/// after `timeout`. Drains stdout on a thread so a full pipe can't deadlock.
/// `GIT_TERMINAL_PROMPT=0` guarantees no credential prompt; the low-speed env
/// aborts a stalled transfer. Returns the captured stdout, or `None` on failure /
/// timeout / non-zero exit.
fn run_bounded_git_ls_remote(url: &str, timeout: Duration) -> Option<String> {
    use std::io::Read;
    let mut child = Command::new("git")
        .args(["ls-remote", "--tags", url])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", "3")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Drain stdout off-thread so the child never blocks writing to a full pipe.
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });

    let start = SystemTime::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                return rx.recv_timeout(Duration::from_millis(200)).ok();
            }
            Ok(None) => {
                if start.elapsed().map(|e| e >= timeout).unwrap_or(true) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

// ── Evaluation core ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct NotifyOutput {
    notify: bool,
    reason: String,
    current_version: String,
    latest_version: Option<String>,
    message: Option<String>,
    update_command: Option<String>,
}

impl NotifyOutput {
    /// Render to JSON, omitting absent optional fields (so the shape matches the
    /// documented example: notify=false carries no message/update_command).
    fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "notify": self.notify,
            "reason": self.reason,
            "current_version": self.current_version,
        });
        if let Some(v) = &self.latest_version {
            obj["latest_version"] = serde_json::json!(v);
        }
        if let Some(m) = &self.message {
            obj["message"] = serde_json::json!(m);
        }
        if let Some(c) = &self.update_command {
            obj["update_command"] = serde_json::json!(c);
        }
        obj
    }
}

fn nonempty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn env_disabled() -> bool {
    std::env::var("AGS_NO_UPDATE_NOTIFIER")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Whether a prior check is still within the throttle window. Primary signal is
/// the local calendar day; if "today" is unavailable, fall back to an
/// epoch-seconds window so a missing `date` still cannot hammer the network.
fn within_throttle(prior: &NotifierState, today: &Option<String>, now: u64) -> bool {
    if let (Some(today), false) = (today, prior.last_checked_date.is_empty()) {
        if let Some(days) = days_between(&prior.last_checked_date, today) {
            return (0..THROTTLE_DAYS).contains(&days);
        }
    }
    prior.checked_at != 0 && now.saturating_sub(prior.checked_at) < (THROTTLE_DAYS as u64) * 86_400
}

/// Pure-ish evaluation against an explicit runtime home (so tests inject a temp
/// dir). Performs at most one network probe, and only when due.
fn evaluate(runtime_home: &Path) -> NotifyOutput {
    let current = AGS_VERSION.to_string();

    if env_disabled() {
        return NotifyOutput {
            notify: false,
            reason: "disabled".to_string(),
            current_version: current,
            latest_version: None,
            message: None,
            update_command: None,
        };
    }

    let prior = read_state(runtime_home);
    let today = today_local();
    let now = unix_now();

    // Throttle: within the window → fresh, NO network, NO state write.
    if let Some(prior) = &prior {
        if within_throttle(prior, &today, now) {
            return NotifyOutput {
                notify: false,
                reason: "fresh".to_string(),
                current_version: current,
                latest_version: nonempty(&prior.latest_version),
                message: None,
                update_command: None,
            };
        }
    }

    // Due: probe (records the attempt date either way so an outage doesn't
    // re-hit GitHub every task).
    let today_str = today.unwrap_or_default();
    let cur_parsed = parse_version(&current);
    match fetch_latest_version() {
        Some(latest) => {
            let latest_str = format!("{}.{}.{}", latest.0, latest.1, latest.2);
            let newer = cur_parsed.map(|c| latest > c).unwrap_or(false);
            let reason = if newer {
                "update-available"
            } else {
                "up-to-date"
            };
            write_state(
                runtime_home,
                &NotifierState {
                    schema_version: STATE_SCHEMA.to_string(),
                    current_version: current.clone(),
                    latest_version: latest_str.clone(),
                    checked_at: now,
                    last_checked_date: today_str,
                    last_result: reason.to_string(),
                },
            );
            if newer {
                NotifyOutput {
                    notify: true,
                    reason: reason.to_string(),
                    current_version: current,
                    latest_version: Some(latest_str.clone()),
                    message: Some(format!(
                        "AGS {latest_str} is available. Run /ags update to update after this task."
                    )),
                    update_command: Some("/ags update".to_string()),
                }
            } else {
                NotifyOutput {
                    notify: false,
                    reason: reason.to_string(),
                    current_version: current,
                    latest_version: Some(latest_str),
                    message: None,
                    update_command: None,
                }
            }
        }
        None => {
            let prev_latest = prior
                .as_ref()
                .map(|p| p.latest_version.clone())
                .unwrap_or_default();
            write_state(
                runtime_home,
                &NotifierState {
                    schema_version: STATE_SCHEMA.to_string(),
                    current_version: current.clone(),
                    latest_version: prev_latest.clone(),
                    checked_at: now,
                    last_checked_date: today_str,
                    last_result: "check-failed".to_string(),
                },
            );
            NotifyOutput {
                notify: false,
                reason: "check-failed".to_string(),
                current_version: current,
                latest_version: nonempty(&prev_latest),
                message: None,
                update_command: None,
            }
        }
    }
}

// ── Read-only status (for `ags update check`; NEVER probes or writes) ───────

/// Read-only notifier status for `ags update check`. Reflects stored state only —
/// it performs NO network probe and writes NOTHING. Due fetch/write belongs to
/// `ags update notify` alone.
pub(in crate::update) fn notifier_status_json(runtime_home: &Path) -> serde_json::Value {
    let state = read_state(runtime_home);
    serde_json::json!({
        "current_version": AGS_VERSION,
        "disabled": env_disabled(),
        "state_present": state.is_some(),
        "latest_version": state.as_ref().map(|s| s.latest_version.clone()).filter(|s| !s.is_empty()),
        "last_checked_date": state.as_ref().map(|s| s.last_checked_date.clone()).filter(|s| !s.is_empty()),
        "last_result": state.as_ref().map(|s| s.last_result.clone()).filter(|s| !s.is_empty()),
        "note": "read-only; run `ags update notify` to perform a throttled check",
    })
}

pub(in crate::update) fn notifier_status_line(runtime_home: &Path) -> String {
    let state = read_state(runtime_home);
    match state {
        Some(s) if !s.last_result.is_empty() => format!(
            "  notifier: last_result={} latest={} last_checked={} (current {})",
            s.last_result,
            if s.latest_version.is_empty() {
                "unknown"
            } else {
                &s.latest_version
            },
            if s.last_checked_date.is_empty() {
                "never"
            } else {
                &s.last_checked_date
            },
            AGS_VERSION,
        ),
        _ => format!(
            "  notifier: no prior check (current {}); run `ags update notify`",
            AGS_VERSION
        ),
    }
}

// ── Command entry ───────────────────────────────────────────────────────────

pub(in crate::update) fn cmd_update_notify(format: &str) {
    let runtime_home = skill_resolver::locate_runtime_home();
    let out = evaluate(&runtime_home);
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&out.to_json()).unwrap_or_else(|_| "{}".to_string())
        );
    } else if out.notify {
        if let Some(m) = &out.message {
            println!("{m}");
        }
    } else {
        println!(
            "AGS update notifier: {} (current {}).",
            out.reason, out.current_version
        );
    }
    // Notifier ALWAYS exits 0 — it must never change a caller's exit code.
}

// ── Tests (hermetic: temp runtime home + fake envs; never real GitHub) ──────

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize env-mutating tests — `std::env::set_var` is process-global.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn tmp(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ags-notify-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn clear_env() {
        for k in [
            "AGS_NO_UPDATE_NOTIFIER",
            "AGS_UPDATE_FAKE_LATEST",
            "AGS_UPDATE_FAKE_FETCH_FAIL",
            "AGS_UPDATE_FAKE_DATE",
            "AGS_UPDATE_SOURCE_URL",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn parse_version_is_strict_and_ignores_prerelease() {
        assert_eq!(parse_version("2.7.0"), Some((2, 7, 0)));
        assert_eq!(parse_version("v2.7.1"), Some((2, 7, 1)));
        // Constraint: 2.10.0 > 2.9.9 (numeric, not string).
        assert!(parse_version("2.10.0").unwrap() > parse_version("2.9.9").unwrap());
        // Pre-release / build / wrong-arity are ignored (None), not coerced.
        assert_eq!(parse_version("v2.8.0-beta"), None);
        assert_eq!(parse_version("2.8.0+meta"), None);
        assert_eq!(parse_version("2.8"), None);
        assert_eq!(parse_version("2.8.0.1"), None);
        assert_eq!(parse_version("not-a-version"), None);
    }

    #[test]
    fn days_between_counts_calendar_days() {
        assert_eq!(days_between("2026-06-19", "2026-06-19"), Some(0));
        assert_eq!(days_between("2026-06-19", "2026-06-26"), Some(7));
        assert_eq!(days_between("2026-06-19", "2026-06-25"), Some(6));
        // Across a month boundary.
        assert_eq!(days_between("2026-06-28", "2026-07-05"), Some(7));
    }

    #[test]
    fn disabled_short_circuits_without_network_or_state() {
        let _g = env_lock();
        clear_env();
        let home = tmp("disabled");
        std::env::set_var("AGS_NO_UPDATE_NOTIFIER", "1");
        let out = evaluate(&home);
        assert!(!out.notify);
        assert_eq!(out.reason, "disabled");
        assert_eq!(out.current_version, AGS_VERSION);
        assert!(!state_path(&home).exists(), "disabled must not write state");
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn fresh_within_window_does_not_probe_or_write() {
        let _g = env_lock();
        clear_env();
        let home = tmp("fresh");
        // Seed state checked "today" so the window is fresh.
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        write_state(
            &home,
            &NotifierState {
                schema_version: STATE_SCHEMA.to_string(),
                current_version: AGS_VERSION.to_string(),
                latest_version: "2.7.0".to_string(),
                checked_at: 111,
                last_checked_date: "2026-06-19".to_string(),
                last_result: "up-to-date".to_string(),
            },
        );
        // A fetch failure injection proves no probe runs when fresh (it would
        // have changed last_result to check-failed otherwise).
        std::env::set_var("AGS_UPDATE_FAKE_FETCH_FAIL", "1");
        let out = evaluate(&home);
        assert!(!out.notify);
        assert_eq!(out.reason, "fresh");
        let after = read_state(&home).unwrap();
        assert_eq!(
            after.last_result, "up-to-date",
            "fresh must not write state"
        );
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn due_with_newer_fake_latest_notifies_and_records() {
        let _g = env_lock();
        clear_env();
        let home = tmp("newer");
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        std::env::set_var("AGS_UPDATE_FAKE_LATEST", "99.0.0");
        let out = evaluate(&home);
        assert!(out.notify);
        assert_eq!(out.reason, "update-available");
        assert_eq!(out.latest_version.as_deref(), Some("99.0.0"));
        assert_eq!(out.update_command.as_deref(), Some("/ags update"));
        assert!(out.message.as_deref().unwrap().contains("99.0.0"));
        let st = read_state(&home).unwrap();
        assert_eq!(st.last_checked_date, "2026-06-19");
        assert_eq!(st.last_result, "update-available");
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn due_with_older_or_equal_is_up_to_date() {
        let _g = env_lock();
        clear_env();
        let home = tmp("equal");
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        std::env::set_var("AGS_UPDATE_FAKE_LATEST", AGS_VERSION);
        let out = evaluate(&home);
        assert!(!out.notify);
        assert_eq!(out.reason, "up-to-date");
        assert_eq!(out.latest_version.as_deref(), Some(AGS_VERSION));
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn due_fetch_failure_is_silent_and_records_attempt() {
        let _g = env_lock();
        clear_env();
        let home = tmp("fail");
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        std::env::set_var("AGS_UPDATE_FAKE_FETCH_FAIL", "1");
        let out = evaluate(&home);
        assert!(!out.notify);
        assert_eq!(out.reason, "check-failed");
        // The attempt date is recorded so a repeated task end does not re-probe.
        let st = read_state(&home).unwrap();
        assert_eq!(st.last_checked_date, "2026-06-19");
        assert_eq!(st.last_result, "check-failed");
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn corrupt_state_is_treated_as_due_no_panic() {
        let _g = env_lock();
        clear_env();
        let home = tmp("corrupt");
        let _ = std::fs::create_dir_all(&home);
        std::fs::write(state_path(&home), "{ not json").unwrap();
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        std::env::set_var("AGS_UPDATE_FAKE_LATEST", "99.0.0");
        let out = evaluate(&home);
        assert!(
            out.notify,
            "corrupt state must be treated as due (no panic)"
        );
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn expired_window_re_probes() {
        let _g = env_lock();
        clear_env();
        let home = tmp("expired");
        write_state(
            &home,
            &NotifierState {
                schema_version: STATE_SCHEMA.to_string(),
                current_version: AGS_VERSION.to_string(),
                latest_version: "2.7.0".to_string(),
                checked_at: 1,
                last_checked_date: "2026-06-01".to_string(),
                last_result: "up-to-date".to_string(),
            },
        );
        // 18 days later → window expired → probe runs (fake newer).
        std::env::set_var("AGS_UPDATE_FAKE_DATE", "2026-06-19");
        std::env::set_var("AGS_UPDATE_FAKE_LATEST", "99.0.0");
        let out = evaluate(&home);
        assert!(out.notify);
        assert_eq!(out.reason, "update-available");
        clear_env();
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn status_json_is_read_only() {
        let _g = env_lock();
        clear_env();
        let home = tmp("status");
        let before = state_path(&home).exists();
        let v = notifier_status_json(&home);
        assert_eq!(v["current_version"], AGS_VERSION);
        assert_eq!(v["state_present"], false);
        // Read-only: no state file created by a status read.
        assert_eq!(state_path(&home).exists(), before);
        let _ = std::fs::remove_dir_all(&home);
    }
}
