# Commercial Use & Licensing

This document explains the commercial boundaries of AGS 2.0 Public Edition in
plain language. It does **not** modify, expand, or override the legal scope of
`LICENSE`. If there is any conflict, `LICENSE` is authoritative.

## What You Can Do

AGS is free for:

- **Personal development** — use AGS to govern your own projects, solo or team.
- **Internal company use** — integrate AGS into your company's private development
  environment, CI pipeline, or internal toolchain. You can use it to improve your
  team's engineering productivity, including in a for-profit company.
- **Evaluation and education** — study AGS, teach AGS workflows, run workshops,
  write about it, compare it with other governance approaches.
- **Modification** — fork AGS, adapt it to your own private workflow, add custom
  policy rules, extend its CLI for internal use.

The key test: **are you using AGS to build your own product, or are you selling
AGS itself?** The first is permitted. The second is not.

## What You Cannot Do (Without Written Permission)

You may **not** do any of the following without prior written permission from
the AGS copyright holders:

- **Sell AGS itself** — as a standalone product, a subscription, a SaaS offering,
  or a marketplace listing.
- **Rebrand and resell** — take AGS, rename it, remove copyright notices, and
  offer it as your own commercial governance product.
- **Shell-sale** — wrap AGS (even a lightly modified fork) as a paid template kit,
  plugin pack, consulting deliverable, or hosted service. The restriction covers
  both the exact AGS codebase and derivatives that are substantially AGS.
- **Build a competing product** — use AGS primarily to create another
  agent-governance suite, task-card governance runtime, or paid multi-agent
  engineering framework.

These restrictions exist because AGS is a governance infrastructure tool. If
vendors could rebrand and sell it as their own product, downstream users would
lose the ability to verify which governance layer they are actually running.

## What Counts as "Your Product" vs "Selling AGS"

This is the practical distinction:

| Scenario | Allowed? |
|---|---|
| Your team uses AGS to manage internal agent workflows | Yes |
| You bundle AGS with your SaaS platform and charge for the platform (not AGS) | Depends / requires license review |
| You write a blog post, course, or book about AGS | Yes |
| You fork AGS, add custom policies for your company, and use it internally | Yes |
| You sell "AGS Pro" on a marketplace | No |
| You offer "AGS-as-a-Service" as a paid subscription | No |
| You build a consulting practice around "Installing AGS" as the primary deliverable | No |
| You rebrand AGS as "Acme Governance Suite" and sell it | No |

AGS can be included as a dependency or toolchain component in a larger product
whose primary value is something other than agent governance only when license
review confirms that the distribution model does not amount to selling AGS
itself. Attribution must be preserved.

## Attribution Requirements

All copies or substantial portions of AGS must retain:

- `LICENSE` — the full license text
- Copyright notice — `Copyright (c) 2025-2026 Agent Governance Suite Contributors`
- `NOTICE.md` — when present
- `THIRD_PARTY_NOTICES.md` — when present

You do not need to display AGS branding in your application's UI, but you must
keep attribution in the source distribution and documentation.

## Trademarks

"Agent Governance Suite" and "AGS" are not registered trademarks, but the
license does not grant permission to use these names (or confusingly similar
marks) to market a different product or service. Truthful attribution and
compatibility statements (e.g., "compatible with AGS task cards") are fine.

## How to Request Commercial Permission

If your use case falls into the restricted category and you want to negotiate a
commercial license:

1. Open an issue on the public repository:
   [github.com/FernandeZ-hjm/agent-governance-suite](https://github.com/FernandeZ-hjm/agent-governance-suite)
2. Describe your intended use case, distribution model, and whether you plan to
   modify AGS.
3. The copyright holders will respond with next steps.

Response time varies. Plan accordingly if your commercial launch depends on
AGS licensing.

## Open Source Dependencies

AGS depends on third-party open-source components, each governed by its own
license. See `THIRD_PARTY_NOTICES.md` for details. A commercial AGS license
would cover AGS itself, not its dependencies — you remain responsible for
complying with dependency licenses.

## Warranty

AGS is provided "as is", without warranty of any kind. See Section 6 of
`LICENSE` for the full disclaimer.
