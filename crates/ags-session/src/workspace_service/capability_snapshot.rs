use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use ags_platform::canonical_workspace_root;

use super::registry_ownership::workspace_key;

/// One immutable capability snapshot per host, loaded once by the workspace daemon.
#[derive(Debug)]
pub struct WorkspaceState {
    root: PathBuf,
    instance_key: String,
    runtime_home: PathBuf,
    catalogs: RwLock<HashMap<String, ags_capability_governance::HostCapabilitySnapshot>>,
}

impl WorkspaceState {
    pub fn new(root: PathBuf, runtime_home: PathBuf) -> Result<Self, String> {
        let instance_key = workspace_key(&root);
        Ok(Self {
            root,
            instance_key,
            runtime_home,
            catalogs: RwLock::new(HashMap::new()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn instance_key(&self) -> &str {
        &self.instance_key
    }

    /// Stable request-binding facts derived from the canonical workspace and
    /// the small set of governance identity files. Daemon cwd and managed
    /// project discovery never participate.
    pub fn project_facts_hash(&self) -> String {
        project_facts_hash_at(&self.root)
    }

    pub fn target_matches(&self, target: &Path) -> bool {
        canonical_workspace_root(target).is_ok_and(|target| target == self.root)
    }

    pub fn loaded_snapshot_hashes(
        &self,
    ) -> Result<std::collections::BTreeMap<String, String>, String> {
        Ok(self
            .catalogs
            .read()
            .map_err(|_| "workspace catalog lock poisoned".to_string())?
            .iter()
            .map(|(host, snapshot)| (host.clone(), snapshot.snapshot_hash.clone()))
            .collect())
    }
}

/// Request-binding facts that change when governance entrypoints or the
/// checked-out commit change. This function is independent of daemon cwd.
pub fn project_facts_hash_at(root: &Path) -> String {
    let mut facts = root.to_string_lossy().as_bytes().to_vec();
    for relative in ["AGENTS.md", "config/agent-project-profile.yaml"] {
        append_bounded_fact(&mut facts, relative, &root.join(relative), 1024 * 1024);
    }
    if let Some(git_dir) = git_directory(root) {
        if let Some(head) = read_bounded_file(&git_dir.join("HEAD"), 4096) {
            append_fact_bytes(&mut facts, "git/HEAD", &head);
            if let Some(reference) = std::str::from_utf8(&head)
                .ok()
                .and_then(|head| head.trim().strip_prefix("ref: "))
            {
                let direct_ref = git_dir.join(reference);
                if read_bounded_file(&direct_ref, 4096).is_some() {
                    append_bounded_fact(&mut facts, "git/ref", &direct_ref, 4096);
                } else if let Some(common_dir) = git_common_directory(&git_dir) {
                    append_bounded_fact(
                        &mut facts,
                        "git/common-ref",
                        &common_dir.join(reference),
                        4096,
                    );
                    append_bounded_fact(
                        &mut facts,
                        "git/packed-refs",
                        &common_dir.join("packed-refs"),
                        1024 * 1024,
                    );
                }
            }
        }
    }
    ags_platform::sha256(facts)
}

fn git_directory(root: &Path) -> Option<PathBuf> {
    let marker = root.join(".git");
    if marker.is_dir() {
        return Some(marker);
    }
    let bytes = read_bounded_file(&marker, 4096)?;
    let value = std::str::from_utf8(&bytes).ok()?.trim();
    let path = PathBuf::from(value.strip_prefix("gitdir: ")?);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn git_common_directory(git_dir: &Path) -> Option<PathBuf> {
    let bytes = read_bounded_file(&git_dir.join("commondir"), 4096)?;
    let path = PathBuf::from(std::str::from_utf8(&bytes).ok()?.trim());
    Some(if path.is_absolute() {
        path
    } else {
        git_dir.join(path)
    })
}

fn append_bounded_fact(facts: &mut Vec<u8>, label: &str, path: &Path, limit: u64) {
    if let Some(bytes) = read_bounded_file(path, limit) {
        append_fact_bytes(facts, label, &bytes);
    }
}

fn append_fact_bytes(facts: &mut Vec<u8>, label: &str, bytes: &[u8]) {
    facts.extend_from_slice(b"\0");
    facts.extend_from_slice(label.as_bytes());
    facts.extend_from_slice(b"\0");
    facts.extend_from_slice(bytes);
}

fn read_bounded_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)
        .ok()?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

impl WorkspaceState {
    /// Validate every candidate before atomically publishing a complete or
    /// partial Host catalog change. Maintenance activation uses this typed seam
    /// so CLI and MCP updates never need recursive subprocesses or self-restarts.
    pub fn activate_host_snapshots(
        &self,
        active_hosts: &[String],
        retired_hosts: &[String],
        replace_all: bool,
    ) -> Result<BTreeMap<String, String>, String> {
        let mut prepared = HashMap::new();
        let mut hashes = BTreeMap::new();
        for host in active_hosts {
            if prepared.contains_key(host) {
                return Err(format!("duplicate Host in capability activation: `{host}`"));
            }
            let (snapshot, _tables) =
                ags_capability_governance::load_static_snapshot(&self.runtime_home, host)
                    .map_err(|error| format!("capability_snapshot_invalid: {error:?}"))?;
            ags_capability_governance::validate_snapshot_authorities(
                &self.runtime_home,
                host,
                &snapshot,
            )?;
            hashes.insert(host.clone(), snapshot.snapshot_hash.clone());
            prepared.insert(host.clone(), snapshot);
        }
        let retired = retired_hosts
            .iter()
            .collect::<std::collections::HashSet<_>>();
        if retired.len() != retired_hosts.len() {
            return Err("duplicate retired Host in capability activation".to_string());
        }
        if active_hosts.iter().any(|host| retired.contains(host)) {
            return Err("capability activation Host cannot be both active and retired".to_string());
        }
        let mut catalogs = self
            .catalogs
            .write()
            .map_err(|_| "workspace catalog lock poisoned".to_string())?;
        if replace_all {
            *catalogs = prepared;
        } else {
            for host in retired_hosts {
                catalogs.remove(host);
            }
            catalogs.extend(prepared);
        }
        Ok(hashes)
    }
}
