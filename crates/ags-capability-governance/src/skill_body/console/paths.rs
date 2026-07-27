use super::*;

pub(super) fn is_safe_path_component(name: &str) -> bool {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return false;
    }
    let mut components = Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(component)), None)
            if component == std::ffi::OsStr::new(name)
    )
}

pub(super) fn supported_skill_hosts() -> Vec<&'static str> {
    SUPPORTED_HOSTS
        .iter()
        .copied()
        .filter(|host| host_skills_subdir(host).is_some())
        .collect()
}

pub(super) fn resolve_source(repo_root: &Path, source: &str) -> PathBuf {
    let path = Path::new(source);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

const CANONICAL_STORES: &[&str] = &["global-skills", "skill-packs"];

pub(super) fn canonical_within_store(repo_root: &Path, canonical_dir: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(canonical_dir) else {
        return false;
    };
    CANONICAL_STORES.iter().any(|store| {
        std::fs::canonicalize(repo_root.join(store))
            .map(|root| real.starts_with(root))
            .unwrap_or(false)
    })
}

pub(super) fn canonical_within_shared_store(home: &Path, name: &str, canonical_dir: &Path) -> bool {
    if !is_safe_path_component(name) {
        return false;
    }
    let shared_root = home.join(".agents/skills");
    let expected = shared_root.join(name);
    match (
        std::fs::canonicalize(canonical_dir),
        std::fs::canonicalize(expected),
        std::fs::canonicalize(shared_root),
    ) {
        (Ok(actual), Ok(expected), Ok(root)) => actual == expected && actual.starts_with(root),
        _ => false,
    }
}

pub(super) fn is_external_shared_skill(
    context: &ConsoleContext,
    capability: &ManagedCapability,
) -> bool {
    let expected = context.home.join(".agents/skills").join(&capability.name);
    matches!(capability.kind, ManagedKind::Skill)
        && matches!(capability.managed_status, ManagedStatus::Governed)
        && capability.source.as_deref().map(Path::new) == Some(expected.as_path())
}

pub(super) fn managed_status_str(status: &ManagedStatus) -> &'static str {
    match status {
        ManagedStatus::SuiteManaged => "suite-managed",
        ManagedStatus::Governed => "governed",
        ManagedStatus::SuiteInterface => "suite-interface",
        ManagedStatus::Discovered => "discovered",
        ManagedStatus::HostSystem => "host-system",
        ManagedStatus::ProjectLocal => "project-local",
        ManagedStatus::Unmanaged => "unmanaged",
        ManagedStatus::RouteTarget => "route-target",
    }
}
