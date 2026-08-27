//! `ags-release` — thin public projection and release tooling (v0.4.21).
//!
//! One command: `release project-public`. `--review` builds and prints the
//! content-addressed plan (writes, retired deletes, blocking findings);
//! `--apply <plan_hash>` applies it transactionally; `--verify` proves S==A
//! and B file integrity. The old v2 `ags release` / `ags verify --scope
//! promotion` surface is replaced by this standalone executable; the ags CLI
//! keeps `release:*` blocked (workspace-local tasks cannot promote).

mod projection;
mod sensitive;
mod spec;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ags-release: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(args: Vec<String>) -> Result<i32, String> {
    if args.first().map(String::as_str) != Some("release")
        || args.get(1).map(String::as_str) != Some("project-public")
    {
        eprintln!(
            "usage: ags-release release project-public --source A --target B [--review | --apply PLAN_HASH | --verify]"
        );
        return Err("unknown command".to_string());
    }
    let mut source: Option<PathBuf> = None;
    let mut target: Option<PathBuf> = None;
    let mut stable: Option<PathBuf> = None;
    let mut mode = "--review".to_string();
    let mut plan_hash: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => {
                i += 1;
                source = args.get(i).map(PathBuf::from);
            }
            "--target" => {
                i += 1;
                target = args.get(i).map(PathBuf::from);
            }
            "--stable" => {
                i += 1;
                stable = args.get(i).map(PathBuf::from);
            }
            "--review" | "--verify" => mode = args[i].clone(),
            "--apply" => {
                mode = "--apply".to_string();
                i += 1;
                plan_hash = args.get(i).cloned();
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 1;
    }
    let source = source.ok_or("--source A is required")?;
    let target = target.ok_or("--target B is required")?;

    match mode.as_str() {
        "--review" => {
            let plan = projection::build_plan(&source, &target);
            // Persist the plan so --apply can consume it by hash.
            let plans_dir = source.join(".ags-release/plans");
            std::fs::create_dir_all(&plans_dir).map_err(|e| e.to_string())?;
            std::fs::write(
                plans_dir.join(format!("{}.json", plan.plan_hash)),
                serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let out = serde_json::to_string_pretty(&plan).map_err(|e| e.to_string())?;
            println!("{out}");
            if !plan.blocking_findings.is_empty() {
                return Ok(1);
            }
            Ok(0)
        }
        "--apply" => {
            let hash = plan_hash.ok_or("--apply requires PLAN_HASH")?;
            let plans_dir = source.join(".ags-release/plans");
            let path = plans_dir.join(format!("{hash}.json"));
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("plan {hash} not found: {e}"))?;
            let plan: projection::Plan =
                serde_json::from_str(&text).map_err(|e| format!("plan parse: {e}"))?;
            let receipt = projection::apply_plan(&plan, &hash)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?
            );
            Ok(if receipt.verified { 0 } else { 1 })
        }
        "--verify" => {
            let stable = stable.ok_or("--verify requires --stable S")?;
            let plans_dir = source.join(".ags-release/plans");
            let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
            if let Ok(entries) = std::fs::read_dir(&plans_dir) {
                for entry in entries.flatten() {
                    let meta = entry.metadata().ok();
                    let mtime = meta.and_then(|m| m.modified().ok());
                    if let Some(mtime) = mtime {
                        let newer = latest.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true);
                        if newer {
                            latest = Some((mtime, entry.path()));
                        }
                    }
                }
            }
            let path = latest
                .map(|(_, p)| p)
                .ok_or("no plan found under .ags-release/plans; run --review first")?;
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let plan: projection::Plan =
                serde_json::from_str(&text).map_err(|e| format!("plan parse: {e}"))?;
            let errors = projection::verify_promotion(&plan, &stable, &source)?;
            if errors.is_empty() {
                println!(
                    "{}",
                    serde_json::json!({"verified": true, "plan_hash": plan.plan_hash})
                );
                Ok(0)
            } else {
                println!(
                    "{}",
                    serde_json::json!({"verified": false, "errors": errors})
                );
                Ok(1)
            }
        }
        other => Err(format!("unknown mode {other}")),
    }
}
