# Third-Party Skills And Upstreams

This public Full Installer keeps the local suite files canonical. Third-party
repositories are used as upstream references, comparison sources, or tool
dependencies. Do not auto-install or overwrite local suite skills from an
upstream repository without review.

## GitHub Author: mattpocock

Upstream repository:

- Git: `https://github.com/mattpocock/skills.git`
- Web: `https://github.com/mattpocock/skills`
- Local policy: crawl upstream, generate a diff proposal, then accept or reject
  changes by human review.

### Required Full Installer Skills

| Local skill | Local path | Upstream path | Relationship |
|---|---|---|---|
| `tdd` | `global-skills/tdd` | `skills/engineering/tdd` | upstream aligned |
| `diagnose` | `global-skills/diagnose` | `skills/engineering/diagnose` | local enriched from upstream |
| `zoom-out` | `global-skills/zoom-out` | `skills/engineering/zoom-out` | local enriched from upstream |
| `caveman-commit` | `global-skills/caveman-commit` | `skills/productivity/caveman` | local split from upstream concept |
| `caveman-review` | `global-skills/caveman-review` | `skills/productivity/caveman` | local split from upstream concept |
| `grill-with-docs` | `global-skills/grill-with-docs` | `skills/engineering/grill-with-docs` | local enriched from upstream |
| `improve-codebase-architecture` | `global-skills/improve-codebase-architecture` | `skills/engineering/improve-codebase-architecture` | local enriched from upstream |
| `prototype` | `global-skills/prototype` | `skills/engineering/prototype` | local enriched from upstream |
| `skill-creator` | `global-skills/skill-creator` | `skills/productivity/write-a-skill` | conceptually related |

### Candidate Skills To Absorb

These are not installed by the current bootstrap flow. They are tracked as
candidate modules for future adoption or task-card integration.

| Candidate | Upstream path | Intended adoption mode | Current decision |
|---|---|---|---|
| `git-guardrails-claude-code` | `skills/misc/git-guardrails-claude-code` | evaluate only | complements existing review gates |
| `to-prd` | `skills/engineering/to-prd` | optional task-card module | useful for PRD shaping |
| `to-issues` | `skills/engineering/to-issues` | optional task-card module | useful for GitHub issue breakdown |
| `triage` | `skills/engineering/triage` | optional task-card module | useful for backlog and issue triage |
| `grill-me` | `skills/productivity/grill-me` | evaluate only | overlaps with `grill-with-docs` |
| `ubiquitous-language` | `skills/deprecated/ubiquitous-language` | evaluate only | deprecated upstream; concept reference only |

## GitHub Author: safishamsi

Upstream repository:

- Git: `https://github.com/safishamsi/graphify.git`
- Web: `https://github.com/safishamsi/graphify`
- Package: `graphifyy`
- CLI: `graphify`

| Local skill | Local path | Relationship |
|---|---|---|
| `graphify` | `global-skills/graphify` | local wrapper around upstream CLI/tool contract |

## Original Suite Skills

The following required skills are tracked as local suite assets in
`manifests/skills-registry.yaml`, not as third-party upstream skills:

- `auto-brainstorm`
- `auto-debug`
- `auto-verify`
- `prompt-maker`
- `claude-delivery-report`
- `finishing-a-development-branch`
- `using-git-worktrees`
- `webapp-testing`
- `database-migration`
- `supply-chain-risk-auditor`
- `superpowers`

## Runtime Mechanism Status

- Hook normalization is included in the public Full Installer through
  `scripts/configure-review-hooks.mjs`, `scripts/bootstrap.sh`, and the runtime
  adapter protocol docs.
- CodeGraph MCP is documented separately in `docs/mcp-servers.md` as a
  supported installable MCP server. It is not installed silently by
  `bootstrap.sh`.
