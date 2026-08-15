---
name: ags-govern
description: Use the contract-v2 capability, Skill, task, evidence, policy, gate, MCP-advice and memory Operations.
---

# AGS Govern

AGS 产品版本：0.4.20

```bash
ags govern capability inventory --host <HostId> --workspace . --format json
ags govern task validate --task-card <task-card> --workspace . --format json
ags govern task plan --task-card <task-card> --workspace . --format json
ags govern capability snapshot --host <HostId> --workspace . --format json
ags govern skill install <skill-id> <local-path> --source-kind local --target-host <HostId> --workspace . --format json
ags govern skill remove <skill-id> --workspace . --format json
ags apply <ACTION_REF> --workspace .
```

Third-party MCP registration remains advice-only.
