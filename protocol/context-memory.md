# Context and memory closure (contract v2)

Project memory is verified continuity, not execution authority. Raw transcripts,
credentials and host-private configuration are outside AGS evidence.

## Read path

Project integration stores a local `memory_uri` in
`config/agent-project-profile.yaml`. A host may read that URI at session start.
Missing memory is a normal empty state and must not trigger workspace guessing.

## Close path

1. Prepare a LaunchPlan with `ags govern task plan --task-card <path>`.
2. Execute and verify within the LaunchPlan authority.
3. Write a Closure schema 1.1 delivery report.
4. Run `ags govern task close --task-card <path> --launch-plan <path> \
   --delivery-report <path> --workspace . --format json`.
5. Consume the returned action with `ags apply <ACTION_REF> --workspace .`.
6. If a separate memory pointer is required, plan it with
   `ags govern memory <receipt> --workspace . --format json`, then apply once.

The close Operation verifies exact task-card, LaunchPlan and delivery-report
hashes before it creates the receipt and workspace-local closure pointer.
Modified or external artifacts fail closed.

## Lifecycle adapter

`ags-host` is the standalone lifecycle executable. Host hooks send typed start,
end and stop-guard events to it. The product parser intentionally has no host or
memory lifecycle subcommands.

Session end may archive only a verified, workspace-bound closure pointer. No
pointer means safe no-op. A host must not infer task completion from a transcript
or LaunchPlan alone.

## Boundaries

- Memory never changes task level, execution mode, topology or review gate.
- Only AGS-owned, exact-hash projection files may be reclaimed.
- User-modified memory and project files are preserved byte-for-byte.
- Context capsules and external host stores are not modified automatically.
- Stable/public promotion and release evidence stay separate from local closure.
