use super::*;
use std::path::Path;

pub(super) fn check_promotion_boundary(
    repo_root: &Path,
    public_root: Option<&Path>,
) -> Vec<CheckItem> {
    let Some(public_root) = public_root else {
        return vec![CheckItem::fail(
            "promotion-public-root-required",
            "promotion",
            "Promotion verification requires an explicit public root.",
            "Pass `--public-root <path>` for the exact public worktree under review.",
        )];
    };
    if !public_root.is_dir() {
        return vec![CheckItem::fail(
            "promotion-public-root",
            "promotion",
            &format!("Public root is not a directory: {}", public_root.display()),
            "Provide an existing public worktree path; promotion never guesses a machine-local path.",
        )];
    }

    let mut items = Vec::new();
    let manifest = crate::release_manifest::verify_promotion_manifest(repo_root, public_root);
    if manifest.passed {
        items.push(CheckItem::pass(
            "promotion-public-manifest",
            "promotion",
            "Explicit public target exactly satisfies the canonical tracked public payload.",
        ));
    } else {
        items.push(CheckItem::fail(
            "promotion-public-manifest",
            "promotion",
            &format!(
                "Public target payload failed: missing=[{}], forbidden=[{}], extra=[{}], content=[{}], authority=[{}]",
                manifest.required_missing.join(", "),
                manifest.forbidden_found.join(", "),
                manifest.extra_files.join(", "),
                manifest.content_mismatches.join(", "),
                manifest.authority_errors.join(", "),
            ),
            "Re-project the public target from manifests/public-release-payload.yaml; do not allowlist non-authority files.",
        ));
    }
    let mut version = check_release_version_surfaces(public_root);
    version.scope = "promotion".to_string();
    version.id = "promotion-version-surfaces".to_string();
    items.push(version);
    items
}
