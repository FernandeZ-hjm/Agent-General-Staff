# AGS MCP contract v2

AGS v0.4.20 exposes exactly two MCP tools:

| Tool | Meaning |
|---|---|
| `ags_decide` | Validate one typed Operation, run a read-only Operation immediately, or return a sealed plan and single-use `action_ref`. |
| `ags_apply` | Consume an `action_ref` in the exact authenticated binding that created it. |

There is no preflight tool, route tool, maintenance tool, resource-as-control
path, or compatibility translation. Natural language stays in the host. Both
tools accept only contract-v2 typed input with unknown fields denied.

## Process model

`ags-mcp stdio` is a lightweight transport and `ags-mcp daemon --workspace
<path>` is the private per-workspace service child. The stdio process does not
own policy state and does not have a mutable current workspace.

```text
one stdio connection
  -> request-scoped WorkspaceResolver
  -> canonical workspace A -> authenticated session A -> control plane A
  -> canonical workspace B -> authenticated session B -> control plane B
```

After the first authenticated handshake, the connection reuses the independent
session for that canonical workspace. A→B→A therefore reuses A without moving
or invalidating B. Daemon process cwd is never a governance identity input.

The private authenticated TCP transport is a bounded message channel. Every
handshake, reply and persistent control frame is limited to 1 MiB. Crossing the
limit terminates that private connection immediately without waiting for a
newline, EOF or drain; an authenticated server emits one structured terminal
error when possible. This is separate from public stdio JSON-RPC framing, which
may drain and resynchronize after rejecting an oversized line.

## Workspace resolution

For `ags_decide`, resolution order is fixed:

1. `operation.request.context.workspace`;
2. one unique declared MCP root;
3. adapter cwd when it is inside exactly one AGS workspace;
4. otherwise `workspace_required` or `workspace_ambiguous`.

MCP roots are discovery hints, never an authorization boundary. A client declares
root support only through `initialize.params.capabilities.roots`. After returning
the initialize response, the server waits for `notifications/initialized`, sends
one correlated `roots/list` request, and accepts only unique canonical `file://`
roots. A negotiated `notifications/roots/list_changed` refresh is coalesced while
one request is pending. A selected root only supplies a candidate path to the
resolver; canonical project facts, registry identity, and the authenticated
daemon handshake must still mint the `WorkspaceBinding`. Private
`initialize.params.roots` input is ignored.

An explicit workspace is resolved independently of declared roots and remains
usable in every discovery state. While a negotiated `roots/list` is pending,
only an explicit workspace is accepted; an omitted workspace returns the
structured pending error and does not fall back to adapter cwd. Once discovery
is unsupported, unavailable, or available with no valid roots, an omitted
workspace may use the allowed adapter-cwd fallback. One valid root wins before
cwd; multiple valid roots are ambiguous and do not fall back. If neither a root
nor cwd resolves, the server returns the structured unavailable or required
error instead of guessing. Client errors, invalid root results, unknown response
IDs, and out-of-order root notifications do not terminate stdio. There is no
HOME, recent-project, fuzzy-path, or managed-projects fallback.

A matching roots response must contain exactly one of a typed `result` or typed
JSON-RPC `error`. Only an explicit matching error transitions discovery to
unavailable. Unknown or mismatched IDs and malformed matching responses do not
mutate discovery state; invalid roots remain untrusted and the request stays
pending rather than becoming available.

`ags_apply` accepts the sealed `action_ref` and optional typed host outcome; it
does not accept a workspace. The stored action route remains sealed to
connection, normalized HostId, canonical workspace, project-facts hash, registry
key, authenticated session, policy, payload, and plan. Wrong workspace, host,
connection, session, replay, or tampering fails closed. Transport and daemon
errors, plus an `awaiting-outcome` / `host_outcome_required` response, retain the
route. A terminal route is deleted only after its bounded response is delivered
successfully, so a post-effect output-budget failure cannot lose the receipt.

## Workspace authority

Canonical path, governed project facts, registry ownership, daemon process
identity, token-authenticated handshake, and the minted session together form a
`WorkspaceBinding`. `managed-projects` may help a human discover projects but
is not identity authority.

Private `initialize.params.roots` input is ignored. Standard MCP roots are
obtained only through the negotiated `roots/list` capability and remain
discovery hints, never workspace authorization.

The external control-plane Interface is `open`, `decide`, and `apply`.
Transport adapters do not assemble commands, invoke nested `ags`, or orchestrate
domain modules. Read-only Operations prove protected state unchanged. Effectful
Operations produce a sealed plan first; only `ags_apply` begins effects.

## Tool and output budgets

- `tools/list` is exactly the two tools above.
- Combined tool input schemas are at most 8 KiB.
- Default JSON responses are at most 16 KiB.
- Large evidence is stored as an integrity-bound artifact and returned through
  a verified `details_uri`; a URI is never emitted without a readable artifact.
  The same authenticated connection reads that URI with standard MCP
  `resources/read`. The internal `details.read` Operation is not accepted by
  `ags_decide` and is never part of the public tool union.

Server information reports product version `0.4.20` and contract v2. Older MCP
tool names, schema IDs, wire envelopes, actions, and leases are invalid input.
