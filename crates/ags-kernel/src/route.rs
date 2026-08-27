//! Lightweight deterministic skill matcher (v0.4.21).
//!
//! Answers "which skill for this need" without an LLM and without touching
//! skill bodies. The route view is DERIVED from the machine skill directory
//! (`~/.agents/skills/*/SKILL.md`): each skill's frontmatter `triggers` list
//! is the single source of routing truth — the same file the host reads for
//! its own description-based selection, so there is never a second set of
//! facts to drift. Matching is pure rule scoring (bounded trigger hits + route
//! priority + trigger specificity). Unique best hit wins; ties abstain and the
//! intent fallback runs only after zero trigger hits. The machine capability
//! lock owns readiness: an unverified match is a candidate, never a skill.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteEntry {
    pub id: String,
    #[serde(default)]
    pub intent_tags: Vec<String>,
    #[serde(default)]
    pub scope_tags: Vec<String>,
    #[serde(default = "default_priority")]
    pub route_priority: u32,
}

fn default_priority() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteView {
    pub schema_version: String,
    pub skills: Vec<RouteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteResult {
    /// A ready skill: unique candidate plus a verified machine body.
    pub skill: Option<String>,
    /// The unique logical candidate before machine readiness is applied.
    /// Present when the intent is clear but the body is missing or stale.
    pub candidate: Option<String>,
    /// True when two or more deterministic trigger hits tie for best score.
    /// Ties always abstain; semantic fallback never breaks them.
    pub ambiguous: bool,
    /// Per-hit evidence: skill id, matched triggers, score.
    pub hits: Vec<Hit>,
    /// Whether `candidate` is currently intact per the machine capability lock.
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub id: String,
    pub matched_tags: Vec<String>,
    pub score: u32,
    pub priority: u32,
}

/// Frontmatter fields AGS derives routing from. `triggers` are natural
/// user phrasings; `route_priority` is optional (defaults to 50).
#[derive(Debug, Default)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub triggers: Vec<String>,
    pub route_priority: Option<u32>,
}

/// Parse the `---`-delimited YAML frontmatter of a SKILL.md for routing
/// fields only (`triggers` list, optional `route_priority`). Handles
/// `name: "quoted"`, `triggers:` followed by `- "item"` lines, and folded
/// descriptions; anything unrecognized is ignored, so AGS stays a reader of
/// the author's file, never a rewriter.
pub fn parse_skill_frontmatter(text: &str) -> SkillFrontmatter {
    let mut fm = SkillFrontmatter::default();
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---") {
        return fm;
    }
    let body = &trimmed[3..];
    let end = body.find("\n---").unwrap_or(body.len());
    let block = &body[..end];
    let mut in_triggers = false;
    for raw in block.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            if in_triggers {
                let item = rest.trim().trim_matches(|c| c == '"' || c == '\'').trim();
                if !item.is_empty() {
                    fm.triggers.push(item.to_string());
                }
            }
            continue;
        }
        // A new top-level key closes the triggers list. This prevents a later
        // YAML list (allowed-tools, resources, etc.) from becoming triggers.
        if !raw.starts_with(char::is_whitespace) {
            in_triggers = false;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            let name = rest.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if !name.is_empty() {
                fm.name = Some(name.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("triggers:") {
            in_triggers = rest.trim().is_empty();
        } else if let Some(rest) = line.strip_prefix("route_priority:") {
            if let Ok(n) = rest.trim().parse::<u32>() {
                fm.route_priority = Some(n);
            }
        }
    }
    fm
}

/// Derive the route view from the machine skill directory: every skill body
/// under `~/.agents/skills/` contributes its frontmatter `triggers` as scope
/// tags, so the route table and the host's skill list share one source of
/// truth (the SKILL.md itself). Skills without triggers simply don't route.
pub fn derive_route_view() -> Result<RouteView> {
    let dir = crate::sync::skills_dir()?;
    let mut skills: Vec<RouteEntry> = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).map_err(|e| crate::error::io("skills_scan_failed", &e))? {
            let entry = entry.map_err(|e| crate::error::io("skills_scan_failed", &e))?;
            let id = entry.file_name().to_string_lossy().to_string();
            let skill_md = entry.path().join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let text = match fs::read_to_string(&skill_md) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let fm = parse_skill_frontmatter(&text);
            if fm.triggers.is_empty() {
                continue;
            }
            skills.push(RouteEntry {
                id,
                intent_tags: vec![],
                scope_tags: fm.triggers,
                route_priority: fm.route_priority.unwrap_or_else(default_priority),
            });
        }
    }
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(RouteView {
        schema_version: "ags://schema/contract/v3/route-view".to_string(),
        skills,
    })
}

/// Load the route view: derived from the machine skill directory.
pub fn load_route_view() -> Result<RouteView> {
    derive_route_view()
}

/// Coarse intent classification for abstract needs the tag matcher cannot see
/// (e.g. "取消这个目录结构" → architecture). Pure rules, no LLM. Consulted
/// ONLY when the deterministic tag matcher has zero hits, so it can never
/// override an explicit tag hit or resolve a deterministic tie.
///
/// Returns (class, matched_cluster_len) so the caller can gate on confidence.
pub fn classify_intent(input: &str) -> Option<(&'static str, usize)> {
    let needle = input.trim().to_lowercase();
    const CLUSTERS: &[(&str, &[&str])] = &[
        (
            "architecture",
            &[
                "这个架构",
                "代码架构",
                "项目架构",
                "目录结构",
                "模块边界",
                "模块怎么拆",
                "重组项目",
                "重构这个模块",
                "重构代码结构",
                "去掉这层",
                "取消这个目录结构",
                "收束这层抽象",
                "设计决策",
                "跨模块设计",
                "codebase architecture",
                "project architecture",
                "project structure",
                "directory structure",
                "module boundaries",
                "split this module",
                "refactor this module",
                "restructure the codebase",
                "remove this layer",
                "simplify this abstraction",
            ],
        ),
        (
            "debug",
            &[
                "诊断问题",
                "调试问题",
                "排查问题",
                "系统崩溃",
                "程序崩溃",
                "应用崩溃",
                "排查崩溃",
                "诊断 bug",
                "调试 bug",
                "性能回归",
                "bug",
                "crash",
                "debug",
            ],
        ),
        (
            "database",
            &[
                "数据库迁移",
                "数据迁移",
                "database migration",
                "migrate database",
                "schema migration",
                "schema change",
                "change schema",
                "zero downtime migration",
            ],
        ),
        (
            "review",
            &[
                "代码审查",
                "审查代码",
                "评审代码",
                "评审改动",
                "代码走查",
                "code review",
                "review code",
                "review this diff",
                "review these changes",
            ],
        ),
        (
            "testing",
            &[
                "浏览器测试",
                "网页测试",
                "本地网页测试",
                "playwright",
                "browser test",
                "webapp test",
                "test local web app",
            ],
        ),
        (
            "merge",
            &[
                "合并冲突",
                "rebase 冲突",
                "merge conflict",
                "rebase conflict",
            ],
        ),
    ];
    let mut best: Option<(&'static str, usize)> = None;
    for (class, words) in CLUSTERS {
        for w in *words {
            if trigger_matches(&needle, w) {
                let len = w.len();
                if best.map(|(_, l)| len > l).unwrap_or(true) {
                    best = Some((class, len));
                }
            }
        }
    }
    best
}

/// Default skill for an intent class. Consulted only as a fallback when the
/// tag matcher is silent; the result still goes through `body_verified`.
pub fn class_default_skill(class: &str) -> Option<&'static str> {
    match class {
        "architecture" => Some("superpowers"),
        "debug" => Some("diagnosing-bugs"),
        "database" => Some("database-migration"),
        "review" => Some("code-review"),
        "testing" => Some("webapp-testing"),
        "merge" => Some("resolving-merge-conflicts"),
        _ => None,
    }
}

fn trigger_matches(input: &str, trigger: &str) -> bool {
    if trigger.is_empty() {
        return false;
    }
    // ASCII identifiers are tokens, not arbitrary substrings: `im` must not
    // match `runtime`, and `db` must not match an unrelated larger word.
    if trigger.chars().all(|c| c.is_ascii_alphanumeric()) {
        return input.match_indices(trigger).any(|(start, _)| {
            let before = input[..start].chars().next_back();
            let end = start + trigger.len();
            let after = input[end..].chars().next();
            let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
            before.map(|c| !is_word(c)).unwrap_or(true)
                && after.map(|c| !is_word(c)).unwrap_or(true)
        });
    }
    // Two-character CJK nouns are too generic for substring routing (`文件`,
    // `任务`, `表格`). Keep exact invocations, otherwise let longer utterance
    // triggers or host-native descriptions decide.
    if trigger.chars().count() <= 2 {
        return input.trim() == trigger;
    }
    input.contains(trigger)
}

/// Deterministic match: trigger hits scored by route priority plus trigger
/// specificity. A unique deterministic hit wins; a tie always abstains. The
/// coarse intent fallback runs only when there are zero trigger hits. A
/// logical candidate becomes `skill` only when its machine body is ready.
pub fn match_route(view: &RouteView, input: &str) -> RouteResult {
    match_route_with(view, input, body_verified)
}

fn match_route_with<F>(view: &RouteView, input: &str, verify: F) -> RouteResult
where
    F: Fn(&str) -> bool,
{
    let needle = input.trim().to_lowercase();
    let mut hits: Vec<Hit> = Vec::new();
    for entry in &view.skills {
        let mut matched_tags: Vec<String> = Vec::new();
        let mut best_score: u32 = 0;
        for tag in entry.intent_tags.iter().chain(entry.scope_tags.iter()) {
            let tag_lower = tag.to_lowercase();
            if trigger_matches(&needle, &tag_lower) {
                let score = entry.route_priority + tag_lower.len() as u32;
                best_score = best_score.max(score);
                matched_tags.push(tag.clone());
            }
        }
        if best_score > 0 {
            hits.push(Hit {
                id: entry.id.clone(),
                matched_tags,
                score: best_score,
                priority: entry.route_priority,
            });
        }
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    let best_score = hits.first().map(|h| h.score);
    let top: Vec<&Hit> = hits
        .iter()
        .filter(|h| Some(h.score) == best_score)
        .collect();
    let ambiguous = top.len() > 1;
    let candidate = if top.len() == 1 {
        top.first().map(|h| h.id.clone())
    } else if top.is_empty() {
        classify_intent(input)
            .and_then(|(class, _)| class_default_skill(class))
            .map(str::to_string)
    } else {
        None
    };
    let verified = candidate.as_deref().map(&verify).unwrap_or(false);
    let skill = candidate.clone().filter(|_| verified);
    RouteResult {
        skill,
        candidate,
        ambiguous,
        hits,
        verified,
    }
}

/// Is the skill body intact per the machine capability lock? The hash is
/// computed on the CURRENT `~/.agents/skills/<id>` entry (following whatever
/// the symlink resolves to right now) — the exact object routing and hosts
/// read — so repointing the link to a different body fails verification.
pub fn body_verified(id: &str) -> bool {
    let Ok(lock) = crate::sync::load_machine_lock() else {
        return false;
    };
    let Some(entry) = lock.entries.iter().find(|e| e.id == id) else {
        return false;
    };
    let Ok(skills) = crate::sync::skills_dir() else {
        return false;
    };
    crate::capabilities::dir_sha256(&skills.join(id))
        .map(|sha| sha == entry.sha256)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_with(entries: Vec<RouteEntry>) -> RouteView {
        RouteView {
            schema_version: "ags://schema/contract/v3/route-view".to_string(),
            skills: entries,
        }
    }

    fn entry(id: &str, tags: &[&str], priority: u32) -> RouteEntry {
        RouteEntry {
            id: id.to_string(),
            intent_tags: tags.iter().map(|s| s.to_string()).collect(),
            scope_tags: vec![],
            route_priority: priority,
        }
    }

    fn ready_route(view: &RouteView, input: &str) -> RouteResult {
        match_route_with(view, input, |_| true)
    }

    #[test]
    fn unique_hit_wins_with_evidence() {
        let view = view_with(vec![
            entry("lark-base", &["多维表格", "base"], 60),
            entry("lark-sheets", &["电子表格", "sheets"], 60),
        ]);
        let r = ready_route(&view, "把记录转成飞书多维表格");
        assert_eq!(r.skill.as_deref(), Some("lark-base"));
        assert!(!r.ambiguous);
        assert!(r.hits[0].matched_tags.contains(&"多维表格".to_string()));
    }

    #[test]
    fn tie_is_ambiguous_rejection() {
        let view = view_with(vec![entry("a", &["表格"], 50), entry("b", &["表格"], 50)]);
        let r = ready_route(&view, "表格");
        assert!(r.ambiguous);
        assert!(r.skill.is_none());
    }

    #[test]
    fn higher_priority_and_specificity_win() {
        let view = view_with(vec![
            entry("generic", &["文档"], 40),
            entry("specific", &["飞书云文档"], 70),
        ]);
        let r = ready_route(&view, "帮我编辑飞书云文档");
        assert_eq!(r.skill.as_deref(), Some("specific"));
        assert_eq!(r.hits.len(), 1, "generic two-character noun must abstain");
    }

    #[test]
    fn no_hit_returns_none() {
        let view = view_with(vec![entry("a", &["表格"], 50)]);
        let r = ready_route(&view, "写一首诗");
        assert!(r.skill.is_none());
        assert!(r.hits.is_empty());
        assert!(!r.verified);
    }

    #[test]
    fn frontmatter_triggers_are_parsed() {
        let md = "\
---
name: demo
description: Demo skill.
triggers:
  - \"诊断 bug\"
  - \"排查崩溃\"
  - debug
route_priority: 60
---
# Demo
";
        let fm = parse_skill_frontmatter(md);
        assert_eq!(fm.name.as_deref(), Some("demo"));
        assert_eq!(fm.triggers, vec!["诊断 bug", "排查崩溃", "debug"]);
        assert_eq!(fm.route_priority, Some(60));
        // A skill without triggers section yields an empty result.
        let plain = "---\nname: x\ndescription: y\n---\n# X\n";
        assert!(parse_skill_frontmatter(plain).triggers.is_empty());
        // Description keywords must not leak into triggers.
        let tricky =
            "---\nname: x\ndescription: \"诊断 bug 的专用技能\"\ntriggers:\n  - 调试\n---\n";
        let fm = parse_skill_frontmatter(tricky);
        assert_eq!(fm.triggers, vec!["调试"]);
        let later_list = "---\nname: x\ntriggers:\n  - route me\nallowed-tools:\n  - bash\n---\n";
        assert_eq!(
            parse_skill_frontmatter(later_list).triggers,
            vec!["route me"]
        );
    }

    #[test]
    fn derive_route_view_reads_skill_md_frontmatter() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let skills = tmp.path().join(".agents/skills");
        for (name, triggers) in [
            ("code-review", vec!["代码审查", "code review"]),
            ("diagnosing-bugs", vec!["诊断 bug", "排查崩溃"]),
        ] {
            let dir = skills.join(name);
            fs::create_dir_all(&dir).unwrap();
            let md = format!(
                "---\nname: {name}\ndescription: test\ntriggers:\n{triggers}\nroute_priority: 55\n---\n# {name}\n",
                triggers = triggers
                    .iter()
                    .map(|t| format!("  - \"{t}\""))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            fs::write(dir.join("SKILL.md"), md).unwrap();
        }
        let view = derive_route_view().unwrap();
        assert_eq!(view.skills.len(), 2, "{view:?}");
        let cr = view.skills.iter().find(|s| s.id == "code-review").unwrap();
        assert_eq!(cr.scope_tags, vec!["代码审查", "code review"]);
        assert_eq!(cr.route_priority, 55);
        let r = ready_route(&view, "帮我代码审查");
        assert_eq!(r.skill.as_deref(), Some("code-review"), "{r:?}");
    }

    #[test]
    fn skills_without_triggers_do_not_route() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let dir = tmp.path().join(".agents/skills/no-trigger");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: no-trigger\ndescription: no triggers\n---\n",
        )
        .unwrap();
        let view = derive_route_view().unwrap();
        assert!(view.skills.is_empty(), "{view:?}");
    }

    #[test]
    fn verified_follows_current_link_target() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let skills = tmp.path().join(".agents/skills");
        let body_a = tmp.path().join("body-a");
        let body_b = tmp.path().join("body-b");
        for (dir, content) in [(&body_a, "# A\n"), (&body_b, "# B\n")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), content).unwrap();
        }
        std::fs::create_dir_all(&skills).unwrap();
        std::os::unix::fs::symlink(&body_a, skills.join("demo")).unwrap();
        crate::sync::sync_bodies().unwrap();
        assert!(body_verified("demo"));
        // Repoint the link to a different body: verification must now fail.
        std::fs::remove_file(skills.join("demo")).unwrap();
        std::os::unix::fs::symlink(&body_b, skills.join("demo")).unwrap();
        assert!(!body_verified("demo"));
    }

    #[test]
    fn intent_class_never_overrides_explicit_tag_hit() {
        let view = view_with(vec![
            entry("lark-mail", &["飞书发邮件"], 60),
            entry("superpowers", &["superpowers"], 30),
        ]);
        // A deterministic utterance wins before the intent fallback.
        let r = ready_route(&view, "帮我用飞书发邮件");
        assert_eq!(r.skill.as_deref(), Some("lark-mail"), "{r:?}");
        let r = ready_route(&view, "superpowers");
        assert_eq!(r.skill.as_deref(), Some("superpowers"), "{r:?}");
    }

    #[test]
    fn short_ascii_trigger_requires_word_boundaries() {
        let view = view_with(vec![entry("lark-im", &["im"], 60)]);
        let unrelated = ready_route(&view, "diagnose AGS runtime");
        assert!(unrelated.skill.is_none(), "{unrelated:?}");
        let explicit = ready_route(&view, "send via im now");
        assert_eq!(explicit.skill.as_deref(), Some("lark-im"), "{explicit:?}");
    }

    #[test]
    fn deterministic_tie_never_uses_intent_fallback() {
        let view = view_with(vec![
            entry("a", &["数据库迁移"], 50),
            entry("b", &["数据库迁移"], 50),
        ]);
        let r = ready_route(&view, "数据库迁移");
        assert!(r.ambiguous, "{r:?}");
        assert!(r.skill.is_none(), "{r:?}");
        assert!(r.candidate.is_none(), "{r:?}");
    }

    #[test]
    fn unverified_unique_hit_is_candidate_not_skill() {
        let view = view_with(vec![entry("demo", &["demo need"], 50)]);
        let r = match_route_with(&view, "demo need", |_| false);
        assert_eq!(r.candidate.as_deref(), Some("demo"));
        assert!(r.skill.is_none());
        assert!(!r.verified);
    }

    #[test]
    fn intent_class_fallback_fires_only_when_tag_matcher_is_silent() {
        let _guard = crate::sync::HOME_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tmp.path());
        let view = view_with(vec![entry("a", &["表格"], 50)]);
        // "取消目录结构" hits no tag; architecture class nominates
        // superpowers, but in this isolated HOME it is not installed, so the
        // fallback abstains (no verified body to route to).
        let r = match_route(&view, "取消这个目录结构");
        assert!(r.skill.is_none(), "{r:?}");
        assert_eq!(r.candidate.as_deref(), Some("superpowers"), "{r:?}");
        assert!(!r.verified);
        // classify_intent itself still sees the architecture class.
        assert_eq!(
            classify_intent("取消这个目录结构").map(|(c, _)| c),
            Some("architecture")
        );
        assert_eq!(
            classify_intent("这个 bug 怎么排查").map(|(c, _)| c),
            Some("debug")
        );
        assert_eq!(
            classify_intent("系统崩溃帮我排查").map(|(c, _)| c),
            Some("debug")
        );
        assert_eq!(classify_intent("写一首诗"), None);
        assert_eq!(classify_intent("design a logo"), None);
        assert_eq!(classify_intent("brainstorm marketing copy"), None);
        assert_eq!(classify_intent("latest release"), None);
        assert_eq!(classify_intent("emergency response"), None);
        assert_eq!(classify_intent("dbadmin account"), None);
        assert_eq!(classify_intent("database connection issue"), None);
        assert_eq!(classify_intent("review this document"), None);
        assert_eq!(classify_intent("write a unit test"), None);
        assert_eq!(classify_intent("merge this branch"), None);
        assert_eq!(
            classify_intent("database migration").map(|(c, _)| c),
            Some("database")
        );
        assert_eq!(
            classify_intent("review these changes").map(|(c, _)| c),
            Some("review")
        );
        assert_eq!(
            classify_intent("test local web app").map(|(c, _)| c),
            Some("testing")
        );
        assert_eq!(
            classify_intent("merge conflict").map(|(c, _)| c),
            Some("merge")
        );
        assert_eq!(
            classify_intent("refactor this module").map(|(c, _)| c),
            Some("architecture")
        );
    }
}
