use super::*;

pub(super) fn plan_hash(plan: &OnboardingPlan) -> Result<String, String> {
    let mut copy = plan.clone();
    copy.plan_hash.clear();
    serde_json::to_vec(&copy)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| format!("cannot hash onboarding plan: {error}"))
}

pub(super) fn pin_github_source(source: &str, revision: &str, subdir: Option<&str>) -> String {
    let mut pinned = format!("{}/tree/{revision}", source.trim_end_matches('/'));
    if let Some(subdir) = subdir.filter(|value| !value.is_empty()) {
        pinned.push('/');
        pinned.push_str(subdir.trim_start_matches('/'));
    }
    pinned
}

pub(super) fn is_git_revision(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub(super) fn normalized_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

pub(super) fn command_in_path(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(command))
        .find(|candidate| candidate.is_file())
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}
