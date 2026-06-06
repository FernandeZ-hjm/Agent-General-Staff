# AGS Examples

This directory contains public-safe synthetic examples that demonstrate AGS
workflows. All content is fabricated — no real task memory, receipts, credentials,
or machine-specific paths are included.

## What You Can Try

### 1. Preflight

```bash
ags session preflight --for claude-code --target .
```

### 2. Task Card Validation

```bash
# Validate a Light task card
bash scripts/validate.sh examples/task-cards/light-demo-task.md

# Validate a Medium task card
bash scripts/validate.sh examples/task-cards/medium-demo-task.md
```

### 3. Policy Resolution

```bash
cargo run -p ags-cli -- policy resolve examples/task-cards/light-demo-task.md
cargo run -p ags-cli -- policy resolve examples/task-cards/medium-demo-task.md
```

### 4. Receipt Verification

```bash
cargo run -p ags-cli -- receipt verify examples/receipts/sample-receipt.json
```

### 5. Demo Project Build

```bash
cargo build --manifest-path examples/demo-project/Cargo.toml
cargo test --manifest-path examples/demo-project/Cargo.toml
```

## Directory Layout

```
examples/
  README.md                     # This file
  demo-project/                 # Minimal synthetic Rust project used by task-card examples
    AGENTS.md                   # Agent entry point
    CLAUDE.md                   # Execution protocol
    Cargo.toml                  # Rust manifest
    src/main.rs                 # Simple CLI
    tests/demo_test.rs          # Basic tests
  task-cards/                   # Valid task cards you can validate
    light-demo-task.md          # Light task (add a function)
    medium-demo-task.md         # Medium task (add a doc)
  outputs/                      # Sample AGS command outputs
    sample-preflight-output.txt # ags session preflight example
    sample-verify-output.txt    # ags verify example
  receipts/                     # Synthetic receipts
    sample-receipt.json         # Receipt referencing light-demo-task.md
```

All examples are self-contained and reference only files within this repository.
