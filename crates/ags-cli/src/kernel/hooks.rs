use crate::cli::HooksAction;
use std::path::PathBuf;

/// `ags hooks install` — install the repo-owned pre-push verification hook.
///
/// Default is a DRY-RUN plan (writes nothing). `--confirm` copies
/// templates/hooks/pre-push.verify.sh into .git/hooks/pre-push and marks it
/// executable on Unix. Never installs silently; uninstall by deleting the file.
fn cmd_hooks_install(confirm: bool) {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let template = root.join("templates/hooks/pre-push.verify.sh");
    let git_hooks_dir = root.join(".git/hooks");
    let dest = git_hooks_dir.join("pre-push");

    if !template.is_file() {
        eprintln!("Template not found: {}", template.display());
        eprintln!("Run `ags hooks install` from the repository root.");
        std::process::exit(1);
    }

    println!("AGS pre-push hook installer");
    println!("  source:      {}", template.display());
    println!("  destination: {}", dest.display());

    if !confirm {
        println!();
        println!("DRY-RUN — nothing was written.");
        if dest.exists() {
            println!(
                "Note: {} already exists; --confirm would overwrite it.",
                dest.display()
            );
        }
        println!("Re-run with --confirm to install:  ags hooks install --confirm");
        println!("Uninstall later with:              rm {}", dest.display());
        return;
    }

    if !git_hooks_dir.is_dir() {
        eprintln!(
            "Not a git working tree (missing {}).",
            git_hooks_dir.display()
        );
        std::process::exit(1);
    }

    let contents = match std::fs::read_to_string(&template) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read template: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(&dest, &contents) {
        eprintln!("Failed to write {}: {e}", dest.display());
        std::process::exit(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }
    }
    println!();
    println!("Installed pre-push hook → {}", dest.display());
    println!("Skip once with:  git push --no-verify");
    println!("Uninstall with:  rm {}", dest.display());
}

pub(crate) fn run(action: HooksAction) {
    match action {
        HooksAction::Install { confirm } => cmd_hooks_install(confirm),
    }
}
