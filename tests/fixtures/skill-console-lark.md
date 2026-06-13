# Fixture: Lark / Feishu capability layering (canonical body + thin index)

Reference example for the `ags skill` management console. It pins the distinct
layers the console must keep separate for one external integration, under the
**canonical body + per-host thin index** model: AGS keeps ONE canonical skill
body; each host gets a symlink thin index pointing back at it. Executable
counterparts live in `crates/skill-governance/src/console.rs`
(`thin_index_preserves_reference_files`, `lark_distinction_is_explicit`,
`codex_*`, `skill_name_mismatch_is_degraded`).

| # | Layer | What it is | `kind` | model field |
|---|---|---|---|---|
| 1 | `lark-cli` (本体) | External official CLI binary talking to the Feishu/Lark Open Platform | `cli-backed` | `canonical_present=false` (AGS holds no body), `managed_status=unmanaged`, `health=degraded` |
| 2 | `lark-*` skill body (e.g. `lark-calendar`) | The ONE canonical skill body AGS manages — `SKILL.md` **plus** `references/` | `skill` | `canonical_present=true`, `managed_status=suite-managed` |
| 3 | Claude Code thin index | `~/.claude/skills/<name>` → symlink → canonical body | — (host visibility) | `host_visibility[claude-code]` |
| 4 | Codex thin index | `~/.codex/skills/<name>` → symlink → canonical body | — (host visibility) | `host_visibility[codex]` |
| 5 | Claude / Codex MCP registration | `claude mcp list` / `codex mcp list` | — | **No `lark` MCP** — Lark is CLI-backed, not MCP-registered |
| 6 | Feishu endpoint health | Live Feishu/Lark API reachability + auth | — (runtime health) | **degraded observation only** — never probed offline; tests never touch network/account |

## Boundaries this fixture enforces

- **One canonical body, many thin indexes.** AGS never copies the skill body per
  host. `adopt`/`update` create a **symlink** thin index at each supported host
  (`~/.claude/skills`, `~/.codex/skills`) pointing at the canonical dir — so
  `lark-calendar/references/…` stays reachable through the host entry. Copying
  only `SKILL.md` (the old bug) would lose `references/` and break the skill at
  runtime.
- **Remove unlinks only the index.** `remove`/`uninstall` move the thin index
  aside to `.bak`; the canonical body is untouched.
- **Body existence ≠ host visibility ≠ endpoint health.** Layer 2
  (`canonical_present`), Layers 3–5 (thin-index / MCP visibility), and Layer 6
  (endpoint reachable) are separate fields.
- **Name must match.** A thin index whose `SKILL.md` front-matter `name` differs
  from the capability name is `degraded`, not visible.
- **AGS never runs external CLIs.** `lark-cli update`, `npx skills`,
  `claude mcp add`/`codex mcp add` are only *advised*, never executed.

## Reproduce against the real machine (read-only / dry-run)

```
ags skill --format json                                      # inventory: canonical + claude-code/codex thin-index visibility
ags skill propose --action adopt --skill lark-calendar       # dry-run: plans a symlink thin index per host → canonical (references travel with it)
ags skill verify  --host claude-code --format json           # thin-index visibility evidence
ags skill verify  --host codex       --format json
```

None of these mutate anything without `--apply`, and none ever run `lark-cli`,
`npx skills`, `claude mcp`, or `codex mcp`.
