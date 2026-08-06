use super::*;

pub(super) fn plan_hash(plan: &OnboardingPlan) -> Result<String, String> {
    let mut copy = plan.clone();
    copy.plan_hash.clear();
    serde_json::to_vec(&copy)
        .map(|bytes| ags_platform::sha256(&bytes))
        .map_err(|error| format!("cannot hash onboarding plan: {error}"))
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
