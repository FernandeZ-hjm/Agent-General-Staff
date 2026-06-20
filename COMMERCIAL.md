# Commercial Use

AGS Public Edition is licensed under the GNU General Public License v3.0 only
(GPL-3.0-only).

This document explains the practical commercial-use posture in plain language.
It does not modify, expand, or override `LICENSE`. If there is any conflict,
`LICENSE` is authoritative.

## What You Can Do

Under GPL-3.0-only, you may:

- use AGS for personal, internal, academic, or commercial engineering work;
- study, modify, and redistribute AGS;
- include AGS in a larger product or internal toolchain.

## Key Condition: Copyleft

If you **distribute** AGS or a derivative work (modified or unmodified), the
distributed version must also be licensed under GPL-3.0-only, and you must
make the corresponding source code available to recipients.

**Internal use** (within your organization, not distributed to third parties)
does not trigger the copyleft obligation.

## What This Means In Practice

| Scenario | GPL obligation triggered? |
|---|---|
| Use AGS internally to govern your AI agents | No |
| Fork AGS, modify it, use it inside your company | No |
| Fork AGS, modify it, distribute it to customers | Yes — must ship source under GPL-3.0 |
| Bundle AGS into a product you sell or distribute | Yes — the bundled portion must be GPL-3.0 |
| Reference AGS task-card format in your own tool | No — formats and protocols are not copyrightable |

## Attribution

Copies or substantial portions of AGS must retain:

- `LICENSE` — the GPL-3.0-only license text;
- the copyright notice: `Copyright (c) 2025-2026 Agent General Staff
  Contributors`;
- `NOTICE.md`, when present;
- `THIRD_PARTY_NOTICES.md`, when present.

Third-party components remain governed by their own licenses. You remain
responsible for complying with dependency and attribution requirements.

## Brand and Endorsement

GPL-3.0-only grants copyright permissions. It does not grant brand endorsement.

You may use "Agent General Staff" or "AGS" for truthful attribution and
compatibility statements, such as "compatible with AGS task cards" or "based on
Agent General Staff".

Do not imply that your product, service, fork, or hosted offering is officially
endorsed by the AGS maintainers unless you have separate written permission.

## Warranty

AGS is provided "as is", without warranty of any kind. See `LICENSE` for the
full disclaimer.
