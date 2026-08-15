//! Descriptor-bound, bounded filesystem observation shared by every Skill
//! source identity consumer in this crate.

#![cfg_attr(not(unix), allow(dead_code))]
#![allow(clippy::unnecessary_cast)] // stat field widths differ per platform

use std::path::{Path, PathBuf};

pub(crate) const MAX_DIRECTORIES: usize = 512;
pub(crate) const MAX_FILES: usize = 512;
pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_NAME_BYTES: usize = 255;
pub(crate) const SKILL_SOURCE_HASH_DOMAIN: &[u8] = b"ags-skill-source-v1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourcePolicy {
    Generic,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservedKind {
    Directory,
    RegularFile,
    Symlink,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObservedNode {
    pub relative_path: String,
    pub kind: ObservedKind,
    pub mode: u32,
    pub bytes: Vec<u8>,
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceObservation {
    pub root_kind: ObservedKind,
    pub root_mode: u32,
    pub root_device: u64,
    pub root_inode: u64,
    pub nodes: Vec<ObservedNode>,
    pub source_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundedRegularFileObservation {
    pub path: PathBuf,
    pub parent: PathBuf,
    pub relative_path: String,
    pub mode: u32,
    pub bytes: Vec<u8>,
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BoundedAbsentObservation {
    pub parent: PathBuf,
    pub relative_path: String,
    pub mode: u32,
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OptionalBoundedRegularFileObservation {
    Present(BoundedRegularFileObservation),
    Absent(BoundedAbsentObservation),
}

#[cfg(unix)]
pub(crate) struct DescriptorRoot {
    path: PathBuf,
    physical_path: PathBuf,
    descriptors: Vec<std::os::fd::OwnedFd>,
    names: Vec<std::ffi::OsString>,
    initial: Vec<rustix::fs::Stat>,
    symlinks: Vec<DescriptorSymlinkBinding>,
}

#[cfg(unix)]
struct DescriptorSymlinkBinding {
    parent: std::os::fd::OwnedFd,
    name: std::ffi::OsString,
    initial: rustix::fs::Stat,
}

#[cfg(all(test, unix))]
thread_local! {
    static AFTER_NAMED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_ROOT_FINAL_FSTAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_LINK_NAMED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_BOUNDED_FILE_NAMED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_BOUNDED_FILE_OPENED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_DESCRIPTOR_SYMLINK_NAMED_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static AFTER_BOUNDED_ABSENT_FIRST_STAT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_named_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_NAMED_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_root_final_fstat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_ROOT_FINAL_FSTAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_link_named_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_LINK_NAMED_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_bounded_file_named_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_BOUNDED_FILE_NAMED_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_bounded_file_opened_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_BOUNDED_FILE_OPENED_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_descriptor_symlink_named_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_DESCRIPTOR_SYMLINK_NAMED_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(all(test, unix))]
pub(crate) fn set_after_bounded_absent_first_stat_hook(hook: Box<dyn FnOnce()>) {
    AFTER_BOUNDED_ABSENT_FIRST_STAT_HOOK.with(|slot| *slot.borrow_mut() = Some(hook));
}

#[cfg(unix)]
mod unix {
    use super::*;
    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Stat};
    use std::fs::File;
    use std::io::Read;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    struct Budget {
        directories: usize,
        files: usize,
        total_bytes: u64,
    }

    impl DescriptorRoot {
        pub(crate) fn open_absolute(path: &Path, label: &str) -> Result<Self, String> {
            use std::collections::VecDeque;
            use std::os::unix::ffi::OsStringExt;
            use std::path::Component;
            const MAX_SYMLINK_DEPTH: usize = 32;
            const MAX_RESOLVED_COMPONENTS: usize = 1024;

            if !path.is_absolute() {
                return Err(format!(
                    "{label} requires an explicit held base for relative paths"
                ));
            }
            let mut pending = VecDeque::new();
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => pending.push_back(name.to_os_string()),
                    _ => return Err(format!("{label} contains a non-normal path component")),
                }
            }
            let root = rustix::fs::open(
                Path::new("/"),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("cannot open filesystem root for {label}: {error}"))?;
            let root_stat = rustix::fs::fstat(&root)
                .map_err(|error| format!("cannot stat filesystem root for {label}: {error}"))?;
            let mut descriptors = vec![root];
            let mut initial = vec![root_stat];
            let mut names = Vec::new();
            let mut symlinks = Vec::new();
            let mut symlink_depth = 0usize;
            let mut resolved_components = 0usize;
            while let Some(name) = pending.pop_front() {
                resolved_components += 1;
                if resolved_components > MAX_RESOLVED_COMPONENTS {
                    return Err(format!(
                        "{label} exceeds {MAX_RESOLVED_COMPONENTS} resolved path components"
                    ));
                }
                let parent = descriptors.last().expect("filesystem root is held");
                let named = rustix::fs::statat(parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(
                    |error| {
                        if error == rustix::io::Errno::NOENT {
                            format!("{label}_not_found")
                        } else {
                            format!("cannot inspect {label} directory component: {error}")
                        }
                    },
                )?;
                let file_type = FileType::from_raw_mode(named.st_mode);
                if file_type == FileType::Symlink {
                    symlink_depth += 1;
                    if symlink_depth > MAX_SYMLINK_DEPTH {
                        return Err(format!("{label} symlink depth exceeds {MAX_SYMLINK_DEPTH}"));
                    }
                    #[cfg(test)]
                    AFTER_DESCRIPTOR_SYMLINK_NAMED_STAT_HOOK.with(|slot| {
                        if let Some(hook) = slot.borrow_mut().take() {
                            hook();
                        }
                    });
                    let target = rustix::fs::readlinkat(parent, &name, Vec::new())
                        .map_err(|error| format!("cannot read {label} symlink: {error}"))?
                        .into_bytes();
                    let named_after = rustix::fs::statat(parent, &name, AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| format!("{label} symlink identity drift"))?;
                    if binding(&named) != binding(&named_after) {
                        return Err(format!("{label} symlink identity drift"));
                    }
                    let held_parent = rustix::io::dup(parent).map_err(|error| {
                        format!("cannot retain held {label} symlink parent: {error}")
                    })?;
                    symlinks.push(DescriptorSymlinkBinding {
                        parent: held_parent,
                        name: name.clone(),
                        initial: named_after,
                    });
                    let target = std::ffi::OsString::from_vec(target);
                    let target_path = PathBuf::from(&target);
                    let absolute_target = target_path.is_absolute();
                    let mut target_components = Vec::new();
                    for component in target_path.components() {
                        match component {
                            Component::RootDir => {}
                            Component::CurDir => {}
                            Component::ParentDir => {
                                target_components.push(std::ffi::OsString::from(".."))
                            }
                            Component::Normal(component) => {
                                target_components.push(component.to_os_string())
                            }
                            Component::Prefix(_) => {
                                return Err(format!("{label} symlink target has a path prefix"))
                            }
                        }
                    }
                    if absolute_target {
                        descriptors.truncate(1);
                        initial.truncate(1);
                        names.clear();
                    }
                    for component in target_components.into_iter().rev() {
                        pending.push_front(component);
                    }
                    continue;
                }
                if name.as_bytes() == b".." {
                    if descriptors.len() == 1 {
                        return Err(format!("{label} symlink traversal escapes filesystem root"));
                    }
                    descriptors.pop();
                    initial.pop();
                    names.pop();
                    continue;
                }
                if file_type != FileType::Directory {
                    return Err(format!("{label} directory component is not a directory"));
                }
                let child = rustix::fs::openat(
                    parent,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| format!("cannot open {label} directory component: {error}"))?;
                let opened = rustix::fs::fstat(&child)
                    .map_err(|error| format!("cannot stat held {label} directory: {error}"))?;
                ensure_same_identity(&named, &opened, label)?;
                descriptors.push(child);
                initial.push(opened);
                names.push(name);
            }
            let physical_path = names.iter().fold(PathBuf::from("/"), |mut path, name| {
                path.push(name);
                path
            });
            let authority = Self {
                path: path.to_path_buf(),
                physical_path,
                descriptors,
                names,
                initial,
                symlinks,
            };
            authority.revalidate(label)?;
            Ok(authority)
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }

        pub(crate) fn physical_path(&self) -> &Path {
            &self.physical_path
        }

        pub(crate) fn descriptor(&self) -> &std::os::fd::OwnedFd {
            self.descriptors.last().expect("filesystem root is held")
        }

        pub(crate) fn open_relative_directory(
            &self,
            relative_path: &Path,
            label: &str,
        ) -> Result<Self, String> {
            self.open_or_create_relative_directory(relative_path, None, label)
        }

        pub(crate) fn create_relative_directory(
            &self,
            relative_path: &Path,
            mode: Mode,
            label: &str,
        ) -> Result<Self, String> {
            self.open_or_create_relative_directory(relative_path, Some(mode), label)
        }

        pub(crate) fn write_relative_file(
            &self,
            relative_path: &Path,
            bytes: &[u8],
            mode: Mode,
            label: &str,
        ) -> Result<(), String> {
            use std::io::Write as _;
            use std::path::Component;
            let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
            let name = relative_path
                .file_name()
                .ok_or_else(|| format!("{label} has no file name"))?;
            let held_parent = if parent.as_os_str().is_empty() {
                self.duplicate(label)?
            } else {
                self.create_relative_directory(parent, Mode::from_raw_mode(0o700), label)?
            };
            if relative_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(format!("{label} contains a non-normal relative path"));
            }
            let fd = rustix::fs::openat(
                held_parent.descriptor(),
                name,
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::TRUNC
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC,
                mode,
            )
            .map_err(|error| format!("cannot open {label} file: {error}"))?;
            let stat = rustix::fs::fstat(&fd)
                .map_err(|error| format!("cannot stat held {label} file: {error}"))?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
                return Err(format!("{label} file is not regular"));
            }
            let mut file = File::from(fd);
            file.write_all(bytes)
                .map_err(|error| format!("cannot write {label} file: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("cannot sync {label} file: {error}"))?;
            held_parent.revalidate(label)?;
            self.revalidate(label)
        }

        pub(crate) fn remove_relative_tree(
            &self,
            relative_path: &Path,
            label: &str,
        ) -> Result<(), String> {
            let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
            let name = relative_path
                .file_name()
                .ok_or_else(|| format!("{label} has no entry name"))?;
            let held_parent = if parent.as_os_str().is_empty() {
                self.duplicate(label)?
            } else {
                self.open_relative_directory(parent, label)?
            };
            remove_entry_at(held_parent.descriptor(), name, label)?;
            held_parent.revalidate(label)?;
            self.revalidate(label)
        }

        pub(crate) fn create_relative_symlink(
            &self,
            relative_path: &Path,
            target: &Path,
            label: &str,
        ) -> Result<(), String> {
            let parent = relative_path.parent().unwrap_or_else(|| Path::new(""));
            let name = relative_path
                .file_name()
                .ok_or_else(|| format!("{label} has no link name"))?;
            let held_parent = if parent.as_os_str().is_empty() {
                self.duplicate(label)?
            } else {
                self.create_relative_directory(parent, Mode::from_raw_mode(0o700), label)?
            };
            rustix::fs::symlinkat(target, held_parent.descriptor(), name)
                .map_err(|error| format!("cannot create {label} symlink: {error}"))?;
            held_parent.revalidate(label)?;
            self.revalidate(label)
        }

        fn open_or_create_relative_directory(
            &self,
            relative_path: &Path,
            create_mode: Option<Mode>,
            label: &str,
        ) -> Result<Self, String> {
            use std::path::Component;
            let mut components = Vec::new();
            for component in relative_path.components() {
                match component {
                    Component::Normal(name) => components.push(name.to_os_string()),
                    _ => return Err(format!("{label} contains a non-normal relative path")),
                }
            }
            if components.is_empty() {
                return Err(format!("{label} has no directory name"));
            }
            let mut duplicate = self.duplicate(label)?;
            for component in components {
                let parent = duplicate
                    .descriptors
                    .last()
                    .expect("held relative root has a descriptor");
                let named = match rustix::fs::statat(parent, &component, AtFlags::SYMLINK_NOFOLLOW)
                {
                    Ok(named) => named,
                    Err(rustix::io::Errno::NOENT) if create_mode.is_some() => {
                        rustix::fs::mkdirat(parent, &component, create_mode.unwrap())
                            .map_err(|error| format!("cannot create {label} directory: {error}"))?;
                        rustix::fs::statat(parent, &component, AtFlags::SYMLINK_NOFOLLOW).map_err(
                            |error| format!("cannot inspect created {label} directory: {error}"),
                        )?
                    }
                    Err(rustix::io::Errno::NOENT) => return Err(format!("{label}_not_found")),
                    Err(error) => return Err(format!("cannot inspect {label} directory: {error}")),
                };
                if FileType::from_raw_mode(named.st_mode) != FileType::Directory {
                    return Err(format!("{label} directory component is not a directory"));
                }
                let child = rustix::fs::openat(
                    parent,
                    &component,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| format!("cannot open {label} directory: {error}"))?;
                let opened = rustix::fs::fstat(&child)
                    .map_err(|error| format!("cannot stat held {label} directory: {error}"))?;
                ensure_same_identity(&named, &opened, label)?;
                duplicate.descriptors.push(child);
                duplicate.initial.push(opened);
                duplicate.names.push(component.clone());
                duplicate.path.push(&component);
                duplicate.physical_path.push(component);
            }
            duplicate.revalidate(label)?;
            Ok(duplicate)
        }

        fn duplicate(&self, label: &str) -> Result<Self, String> {
            let descriptors = self
                .descriptors
                .iter()
                .map(|descriptor| {
                    rustix::io::dup(descriptor).map_err(|error| {
                        format!("cannot duplicate held {label} directory: {error}")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let symlinks = self
                .symlinks
                .iter()
                .map(|symlink| {
                    Ok(DescriptorSymlinkBinding {
                        parent: rustix::io::dup(&symlink.parent).map_err(|error| {
                            format!("cannot duplicate held {label} symlink parent: {error}")
                        })?,
                        name: symlink.name.clone(),
                        initial: symlink.initial,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Self {
                path: self.path.clone(),
                physical_path: self.physical_path.clone(),
                descriptors,
                names: self.names.clone(),
                initial: self.initial.clone(),
                symlinks,
            })
        }

        pub(crate) fn revalidate(&self, label: &str) -> Result<(), String> {
            for (index, initial) in self.initial.iter().enumerate() {
                let opened = rustix::fs::fstat(&self.descriptors[index]).map_err(|error| {
                    format!("cannot revalidate held {label} directory: {error}")
                })?;
                if identity(initial) != identity(&opened) {
                    return Err(format!("{label}_root_identity_drift"));
                }
                if index > 0 {
                    let named = rustix::fs::statat(
                        &self.descriptors[index - 1],
                        &self.names[index - 1],
                        AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .map_err(|_| format!("{label}_root_identity_drift"))?;
                    if identity(initial) != identity(&named) {
                        return Err(format!("{label}_root_identity_drift"));
                    }
                }
            }
            for symlink in &self.symlinks {
                let named =
                    rustix::fs::statat(&symlink.parent, &symlink.name, AtFlags::SYMLINK_NOFOLLOW)
                        .map_err(|_| format!("{label} symlink identity drift"))?;
                if binding(&symlink.initial) != binding(&named) {
                    return Err(format!("{label} symlink identity drift"));
                }
            }
            Ok(())
        }
    }

    fn remove_entry_at(
        parent: &std::os::fd::OwnedFd,
        name: &std::ffi::OsStr,
        label: &str,
    ) -> Result<(), String> {
        let stat = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot inspect {label} cleanup entry: {error}"))?;
        if FileType::from_raw_mode(stat.st_mode) == FileType::Directory {
            let child = rustix::fs::openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("cannot open {label} cleanup directory: {error}"))?;
            let mut names = Dir::read_from(&child)
                .map_err(|error| format!("cannot enumerate {label} cleanup directory: {error}"))?
                .filter_map(|entry| entry.ok())
                .map(|entry| std::ffi::OsString::from_vec(entry.file_name().to_bytes().to_vec()))
                .filter(|entry| entry.as_bytes() != b"." && entry.as_bytes() != b"..")
                .collect::<Vec<_>>();
            names.sort();
            for child_name in names {
                remove_entry_at(&child, &child_name, label)?;
            }
            rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR)
                .map_err(|error| format!("cannot remove {label} directory: {error}"))?;
        } else {
            rustix::fs::unlinkat(parent, name, AtFlags::empty())
                .map_err(|error| format!("cannot remove {label} entry: {error}"))?;
        }
        Ok(())
    }

    pub(super) fn observe_bounded_file(
        path: &Path,
        maximum_bytes: u64,
        label: &str,
    ) -> Result<BoundedRegularFileObservation, String> {
        if !path.is_absolute() {
            return Err(format!(
                "{label} requires an explicit held base for relative paths"
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| format!("{label} has no parent: {}", path.display()))?;
        let root = DescriptorRoot::open_absolute(parent, label)?;
        let name = path
            .file_name()
            .ok_or_else(|| format!("{label} has no file name: {}", path.display()))?;
        match observe_optional_bounded_file_at(&root, Path::new(name), maximum_bytes, label)? {
            OptionalBoundedRegularFileObservation::Present(observed) => Ok(observed),
            OptionalBoundedRegularFileObservation::Absent(_) => Err(format!("{label}_not_found")),
        }
    }

    pub(super) fn observe_optional_bounded_file_at(
        root: &DescriptorRoot,
        relative_path: &Path,
        maximum_bytes: u64,
        label: &str,
    ) -> Result<OptionalBoundedRegularFileObservation, String> {
        use std::path::Component;
        let mut components = Vec::new();
        for component in relative_path.components() {
            match component {
                Component::Normal(name) => components.push(name.to_os_string()),
                _ => return Err(format!("{label} contains a non-normal relative path")),
            }
        }
        let name = components
            .pop()
            .ok_or_else(|| format!("{label} has no file name"))?;
        let mut current = rustix::io::dup(root.descriptor())
            .map_err(|error| format!("cannot duplicate held {label} root: {error}"))?;
        let mut held_directories = Vec::new();
        for (index, component) in components.iter().enumerate() {
            let named = match rustix::fs::statat(&current, component, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(named) => named,
                Err(rustix::io::Errno::NOENT) => {
                    let mut suffix = components[index..].iter().collect::<PathBuf>();
                    suffix.push(&name);
                    let parent = root
                        .physical_path()
                        .join(components[..index].iter().collect::<PathBuf>());
                    let absent = confirm_bounded_absence(
                        root,
                        &current,
                        &held_directories,
                        component,
                        &parent,
                        &suffix,
                        label,
                    )?;
                    return Ok(OptionalBoundedRegularFileObservation::Absent(absent));
                }
                Err(error) => return Err(format!("cannot inspect {label} parent: {error}")),
            };
            if FileType::from_raw_mode(named.st_mode) != FileType::Directory {
                return Err(format!("{label} parent is not a directory"));
            }
            let child = rustix::fs::openat(
                &current,
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("cannot open {label} parent: {error}"))?;
            let opened = rustix::fs::fstat(&child)
                .map_err(|error| format!("cannot stat held {label} parent: {error}"))?;
            ensure_same_identity(&named, &opened, label)?;
            held_directories.push((current, component.clone(), opened));
            current = child;
        }
        let parent_fd = current;
        let parent_opened_before = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot stat held {label} parent: {error}"))?;
        let parent = if components.is_empty() {
            root.physical_path().to_path_buf()
        } else {
            root.physical_path()
                .join(components.iter().collect::<PathBuf>())
        };
        let absolute = parent.join(&name);
        let named_before = match rustix::fs::statat(&parent_fd, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => {
                let absent = confirm_bounded_absence(
                    root,
                    &parent_fd,
                    &held_directories,
                    &name,
                    &parent,
                    Path::new(&name),
                    label,
                )?;
                return Ok(OptionalBoundedRegularFileObservation::Absent(absent));
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect {label} {}: {error}",
                    absolute.display()
                ))
            }
        };
        #[cfg(test)]
        AFTER_BOUNDED_FILE_NAMED_STAT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        if FileType::from_raw_mode(named_before.st_mode) != FileType::RegularFile {
            return Err(format!(
                "{label} must be a regular file: {}",
                absolute.display()
            ));
        }
        let named_size = u64::try_from(named_before.st_size).unwrap_or(u64::MAX);
        if named_size > maximum_bytes {
            return Err(format!("{label} exceeds {maximum_bytes} bytes"));
        }
        let file_fd = rustix::fs::openat(
            &parent_fd,
            &name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| format!("cannot open {label} {}: {error}", absolute.display()))?;
        let opened_before = rustix::fs::fstat(&file_fd)
            .map_err(|error| format!("cannot stat held {label}: {error}"))?;
        if FileType::from_raw_mode(opened_before.st_mode) != FileType::RegularFile
            || binding(&named_before) != binding(&opened_before)
        {
            return Err(format!("{label}_read_input_drift"));
        }
        let opened_size = u64::try_from(opened_before.st_size).unwrap_or(u64::MAX);
        if opened_size > maximum_bytes {
            return Err(format!("{label} exceeds {maximum_bytes} bytes"));
        }
        #[cfg(test)]
        AFTER_BOUNDED_FILE_OPENED_STAT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        let file = File::from(file_fd);
        let mut limited = file.take(maximum_bytes + 1);
        let mut bytes = Vec::with_capacity(usize::try_from(opened_size).unwrap_or(0));
        limited
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {label} {}: {error}", absolute.display()))?;
        if bytes.len() as u64 > maximum_bytes {
            return Err(format!("{label} exceeds {maximum_bytes} bytes"));
        }
        let file = limited.into_inner();
        let opened_after = rustix::fs::fstat(&file)
            .map_err(|error| format!("cannot revalidate held {label}: {error}"))?;
        let named_after = rustix::fs::statat(&parent_fd, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot revalidate {label} path: {error}"))?;
        let parent_opened_after = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot revalidate held {label} parent: {error}"))?;
        if binding(&opened_before) != binding(&opened_after)
            || binding(&opened_before) != binding(&named_after)
            || u64::try_from(opened_before.st_size).ok() != Some(bytes.len() as u64)
            || identity(&parent_opened_before) != identity(&parent_opened_after)
        {
            return Err(format!("{label}_read_input_drift"));
        }
        let mut opened_child = &parent_fd;
        for (held_parent, child_name, initial) in held_directories.iter().rev() {
            let named = rustix::fs::statat(held_parent, child_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| format!("{label}_read_input_drift"))?;
            let opened =
                rustix::fs::fstat(opened_child).map_err(|_| format!("{label}_read_input_drift"))?;
            if identity(initial) != identity(&named) || identity(initial) != identity(&opened) {
                return Err(format!("{label}_read_input_drift"));
            }
            opened_child = held_parent;
        }
        root.revalidate(label)?;
        Ok(OptionalBoundedRegularFileObservation::Present(
            BoundedRegularFileObservation {
                path: absolute,
                parent,
                relative_path: name.to_string_lossy().into_owned(),
                mode: mode(&opened_before),
                bytes,
                device: opened_before.st_dev as u64,
                inode: opened_before.st_ino,
            },
        ))
    }

    fn confirm_bounded_absence(
        root: &DescriptorRoot,
        parent_fd: &std::os::fd::OwnedFd,
        held_directories: &[(std::os::fd::OwnedFd, std::ffi::OsString, Stat)],
        first_absent_name: &std::ffi::OsStr,
        parent: &Path,
        suffix: &Path,
        label: &str,
    ) -> Result<BoundedAbsentObservation, String> {
        let parent_before = rustix::fs::fstat(parent_fd)
            .map_err(|error| format!("cannot stat held {label} absent parent: {error}"))?;
        #[cfg(test)]
        AFTER_BOUNDED_ABSENT_FIRST_STAT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        match rustix::fs::statat(parent_fd, first_absent_name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {}
            _ => return Err(format!("{label}_absent_component_appeared")),
        }
        let parent_after = rustix::fs::fstat(parent_fd)
            .map_err(|error| format!("cannot revalidate held {label} absent parent: {error}"))?;
        if identity(&parent_before) != identity(&parent_after) {
            return Err(format!("{label}_absent_parent_identity_drift"));
        }
        let mut opened_child = parent_fd;
        for (held_parent, child_name, initial) in held_directories.iter().rev() {
            let named = rustix::fs::statat(held_parent, child_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| format!("{label}_absent_parent_identity_drift"))?;
            let opened = rustix::fs::fstat(opened_child)
                .map_err(|_| format!("{label}_absent_parent_identity_drift"))?;
            if identity(initial) != identity(&named) || identity(initial) != identity(&opened) {
                return Err(format!("{label}_absent_parent_identity_drift"));
            }
            opened_child = held_parent;
        }
        root.revalidate(label)?;
        Ok(BoundedAbsentObservation {
            parent: parent.to_path_buf(),
            relative_path: suffix.to_string_lossy().replace('\\', "/"),
            mode: mode(&parent_after),
            device: parent_after.st_dev as u64,
            inode: parent_after.st_ino,
        })
    }

    pub(super) fn observe(path: &Path, policy: SourcePolicy) -> Result<SourceObservation, String> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| format!("skill source has no file name: {}", path.display()))?;
        let parent_fd = rustix::fs::open(
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            format!(
                "cannot open skill source parent {}: {error}",
                parent.display()
            )
        })?;
        let parent_before = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot stat skill source parent: {error}"))?;
        let named_before = rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot inspect skill source {}: {error}", path.display()))?;
        let kind = FileType::from_raw_mode(named_before.st_mode);
        let mut canonical = SKILL_SOURCE_HASH_DOMAIN.to_vec();
        let mut nodes = Vec::new();
        let mut budget = Budget {
            directories: 0,
            files: 0,
            total_bytes: 0,
        };

        if kind == FileType::Directory {
            let root_fd = rustix::fs::openat(
                &parent_fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| {
                format!("cannot open skill source root {}: {error}", path.display())
            })?;
            let opened_before = rustix::fs::fstat(&root_fd)
                .map_err(|error| format!("cannot stat skill source root fd: {error}"))?;
            ensure_same(&named_before, &opened_before, "skill source root")?;
            scan_directory(
                &root_fd,
                Path::new(""),
                policy,
                &mut budget,
                &mut nodes,
                &mut canonical,
            )?;
            let opened_after = rustix::fs::fstat(&root_fd)
                .map_err(|error| format!("cannot revalidate skill source root fd: {error}"))?;
            #[cfg(test)]
            AFTER_ROOT_FINAL_FSTAT_HOOK.with(|slot| {
                if let Some(hook) = slot.borrow_mut().take() {
                    hook();
                }
            });
            let named_after = rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate skill source root: {error}"))?;
            ensure_same(&opened_before, &opened_after, "skill source root")?;
            ensure_same(&opened_before, &named_after, "skill source root")?;
        } else {
            let relative = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .ok_or_else(|| "skill source file has no name".to_string())?;
            observe_node(
                &parent_fd,
                name,
                &relative,
                policy,
                &mut budget,
                &mut nodes,
                &mut canonical,
                Some(named_before),
            )?;
        }
        let parent_after = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot revalidate skill source parent: {error}"))?;
        ensure_same_identity(&parent_before, &parent_after, "skill source parent")?;
        let root_mode = mode(&named_before);
        let root_kind = if kind == FileType::Directory {
            ObservedKind::Directory
        } else if kind == FileType::RegularFile {
            ObservedKind::RegularFile
        } else if kind == FileType::Symlink {
            ObservedKind::Symlink
        } else {
            return Err(format!("special_file_refused: {}", path.display()));
        };
        let observation = SourceObservation {
            root_kind,
            root_mode,
            root_device: named_before.st_dev as u64,
            root_inode: named_before.st_ino as u64,
            nodes,
            source_hash: ags_platform::sha256(canonical),
        };
        Ok(observation)
    }

    pub(super) fn observe_directory_at(
        root: &DescriptorRoot,
        relative_path: &Path,
        policy: SourcePolicy,
    ) -> Result<SourceObservation, String> {
        let held = if relative_path.as_os_str().is_empty() {
            root.duplicate("skill source root")?
        } else {
            root.open_relative_directory(relative_path, "skill source root")?
        };
        let opened_before = rustix::fs::fstat(held.descriptor())
            .map_err(|error| format!("cannot stat held skill source root: {error}"))?;
        let mut canonical = SKILL_SOURCE_HASH_DOMAIN.to_vec();
        let mut nodes = Vec::new();
        let mut budget = Budget {
            directories: 0,
            files: 0,
            total_bytes: 0,
        };
        scan_directory(
            held.descriptor(),
            Path::new(""),
            policy,
            &mut budget,
            &mut nodes,
            &mut canonical,
        )?;
        let opened_after = rustix::fs::fstat(held.descriptor())
            .map_err(|error| format!("cannot revalidate held skill source root: {error}"))?;
        ensure_same(&opened_before, &opened_after, "skill source root")?;
        held.revalidate("skill source root")?;
        root.revalidate("skill source authority")?;
        Ok(SourceObservation {
            root_kind: ObservedKind::Directory,
            root_mode: mode(&opened_before),
            root_device: opened_before.st_dev as u64,
            root_inode: opened_before.st_ino,
            nodes,
            source_hash: ags_platform::sha256(canonical),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_directory(
        directory_fd: &impl std::os::fd::AsFd,
        relative_directory: &Path,
        policy: SourcePolicy,
        budget: &mut Budget,
        nodes: &mut Vec<ObservedNode>,
        canonical: &mut Vec<u8>,
    ) -> Result<(), String> {
        let directory_before = rustix::fs::fstat(directory_fd)
            .map_err(|error| format!("cannot stat candidate directory fd: {error}"))?;
        let mut names = Vec::new();
        for entry in Dir::read_from(directory_fd)
            .map_err(|error| format!("cannot duplicate candidate directory fd: {error}"))?
        {
            let entry = entry
                .map_err(|error| format!("cannot enumerate candidate directory fd: {error}"))?;
            let bytes = entry.file_name().to_bytes();
            if matches!(bytes, b"." | b"..") {
                continue;
            }
            if names.len() >= MAX_FILES + MAX_DIRECTORIES {
                return Err("skill source entry budget exceeded".to_string());
            }
            names.push(bytes.to_vec());
        }
        names.sort();
        for name_bytes in names {
            if name_bytes.len() > MAX_NAME_BYTES {
                return Err(format!("skill source name exceeds {MAX_NAME_BYTES} bytes"));
            }
            let name = std::ffi::OsStr::from_bytes(&name_bytes);
            let relative_path = relative_directory.join(name);
            let relative = match policy {
                SourcePolicy::Strict => relative_path
                    .to_str()
                    .map(|value| value.replace('\\', "/"))
                    .ok_or_else(|| "candidate path is not valid UTF-8".to_string())?,
                SourcePolicy::Generic => relative_path.to_string_lossy().replace('\\', "/"),
            };
            observe_node(
                directory_fd,
                name,
                &relative,
                policy,
                budget,
                nodes,
                canonical,
                None,
            )?;
        }
        let directory_after = rustix::fs::fstat(directory_fd)
            .map_err(|error| format!("cannot revalidate candidate directory fd: {error}"))?;
        ensure_same(&directory_before, &directory_after, "candidate directory")
            .map_err(|_| "candidate_read_input_drift_during_materialization".to_string())?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn observe_node(
        parent_fd: &impl std::os::fd::AsFd,
        name: &std::ffi::OsStr,
        relative: &str,
        policy: SourcePolicy,
        budget: &mut Budget,
        nodes: &mut Vec<ObservedNode>,
        canonical: &mut Vec<u8>,
        known_stat: Option<Stat>,
    ) -> Result<(), String> {
        let named_before = match known_stat {
            Some(stat) => stat,
            None => rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot inspect candidate {relative}: {error}"))?,
        };
        #[cfg(test)]
        AFTER_NAMED_STAT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        let kind = FileType::from_raw_mode(named_before.st_mode);
        if kind == FileType::Directory {
            budget.directories += 1;
            if budget.directories > MAX_DIRECTORIES {
                return Err(format!(
                    "skill source directory count exceeds {MAX_DIRECTORIES} directories"
                ));
            }
            let child_fd = rustix::fs::openat(
                parent_fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("cannot open candidate directory {relative}: {error}"))?;
            let opened_before = rustix::fs::fstat(&child_fd)
                .map_err(|error| format!("cannot stat candidate directory {relative}: {error}"))?;
            if FileType::from_raw_mode(opened_before.st_mode) != FileType::Directory
                || binding(&named_before) != binding(&opened_before)
            {
                return Err("candidate_read_input_drift_during_materialization".to_string());
            }
            canonical.extend_from_slice(b"D\0");
            canonical.extend_from_slice(relative.as_bytes());
            canonical.push(0);
            nodes.push(node(
                relative,
                ObservedKind::Directory,
                &opened_before,
                Vec::new(),
            ));
            scan_directory(
                &child_fd,
                Path::new(relative),
                policy,
                budget,
                nodes,
                canonical,
            )?;
            let opened_after = rustix::fs::fstat(&child_fd).map_err(|error| {
                format!("cannot revalidate candidate directory {relative}: {error}")
            })?;
            let named_after = rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate candidate {relative}: {error}"))?;
            if binding(&opened_before) != binding(&opened_after)
                || binding(&opened_before) != binding(&named_after)
            {
                return Err("candidate_read_input_drift_during_materialization".to_string());
            }
        } else if kind == FileType::RegularFile {
            budget.files += 1;
            if budget.files > MAX_FILES {
                return Err(format!("skill source exceeds {MAX_FILES} files"));
            }
            let named_size = u64::try_from(named_before.st_size).unwrap_or(u64::MAX);
            if named_size > MAX_FILE_BYTES {
                return Err(format!(
                    "skill source file exceeds {MAX_FILE_BYTES} bytes: {relative}"
                ));
            }
            let file_fd = rustix::fs::openat(
                parent_fd,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| format!("cannot open candidate file {relative}: {error}"))?;
            let opened_before = rustix::fs::fstat(&file_fd)
                .map_err(|error| format!("cannot stat candidate file {relative}: {error}"))?;
            if FileType::from_raw_mode(opened_before.st_mode) != FileType::RegularFile
                || named_before.st_dev != opened_before.st_dev
                || named_before.st_ino != opened_before.st_ino
            {
                return Err("candidate_read_input_drift_during_materialization".to_string());
            }
            let file = File::from(file_fd);
            let mut limited = file.take(MAX_FILE_BYTES + 1);
            let mut bytes = Vec::with_capacity(usize::try_from(named_size).unwrap_or(0));
            limited
                .read_to_end(&mut bytes)
                .map_err(|error| format!("cannot read candidate {relative}: {error}"))?;
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err(format!(
                    "skill source file exceeds {MAX_FILE_BYTES} bytes: {relative}"
                ));
            }
            budget.total_bytes = budget.total_bytes.saturating_add(bytes.len() as u64);
            if budget.total_bytes > MAX_TOTAL_BYTES {
                return Err(format!(
                    "skill source exceeds {MAX_TOTAL_BYTES} total bytes"
                ));
            }
            let file = limited.into_inner();
            let opened_after = rustix::fs::fstat(&file)
                .map_err(|error| format!("cannot revalidate candidate file {relative}: {error}"))?;
            let named_after = rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate candidate {relative}: {error}"))?;
            if binding(&opened_before) != binding(&opened_after)
                || binding(&opened_before) != binding(&named_after)
                || u64::try_from(opened_before.st_size).ok() != Some(bytes.len() as u64)
            {
                return Err("candidate_read_input_drift_during_materialization".to_string());
            }
            canonical.extend_from_slice(b"F\0");
            canonical.extend_from_slice(relative.as_bytes());
            canonical.push(0);
            canonical.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            canonical.extend_from_slice(&bytes);
            nodes.push(node(
                relative,
                ObservedKind::RegularFile,
                &opened_before,
                bytes,
            ));
        } else if kind == FileType::Symlink && policy == SourcePolicy::Generic {
            #[cfg(test)]
            AFTER_LINK_NAMED_STAT_HOOK.with(|slot| {
                if let Some(hook) = slot.borrow_mut().take() {
                    hook();
                }
            });
            let target = rustix::fs::readlinkat(parent_fd, name, Vec::new())
                .map_err(|error| format!("cannot read link {relative}: {error}"))?
                .into_bytes();
            if target.len() as u64 > MAX_FILE_BYTES {
                return Err(format!(
                    "skill source link target exceeds {MAX_FILE_BYTES} bytes: {relative}"
                ));
            }
            budget.total_bytes = budget.total_bytes.saturating_add(target.len() as u64);
            if budget.total_bytes > MAX_TOTAL_BYTES {
                return Err(format!(
                    "skill source exceeds {MAX_TOTAL_BYTES} total bytes"
                ));
            }
            let named_after = rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| format!("cannot revalidate link {relative}: {error}"))?;
            if binding(&named_before) != binding(&named_after) {
                return Err("skill_source_link_drift_during_observation".to_string());
            }
            canonical.extend_from_slice(b"L\0");
            canonical.extend_from_slice(relative.as_bytes());
            canonical.push(0);
            canonical.extend_from_slice(String::from_utf8_lossy(&target).as_bytes());
            canonical.push(0);
            nodes.push(node(relative, ObservedKind::Symlink, &named_before, target));
        } else if kind == FileType::Symlink {
            return Err(format!("symlink_refused: {relative}"));
        } else {
            return Err(format!("special_file_refused: {relative}"));
        }
        Ok(())
    }

    fn node(relative: &str, kind: ObservedKind, stat: &Stat, bytes: Vec<u8>) -> ObservedNode {
        ObservedNode {
            relative_path: relative.to_string(),
            kind,
            mode: mode(stat),
            bytes,
            device: stat.st_dev as u64,
            inode: stat.st_ino,
        }
    }

    pub(super) fn observe_link(path: &Path) -> Result<Option<Vec<u8>>, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "host index has no parent".to_string())?;
        let name = path
            .file_name()
            .ok_or_else(|| "host index has no file name".to_string())?;
        let parent_fd = match rustix::fs::open(
            parent,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "cannot open host index parent {}: {error}",
                    parent.display()
                ))
            }
        };
        let parent_before = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot stat host index parent: {error}"))?;
        let before = match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => stat,
            Err(rustix::io::Errno::NOENT) => {
                match rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                    Err(rustix::io::Errno::NOENT) => {}
                    _ => {
                        return Err(format!(
                            "host_link_appeared_during_observation: {}",
                            path.display()
                        ))
                    }
                }
                let parent_after = rustix::fs::fstat(&parent_fd)
                    .map_err(|error| format!("cannot revalidate host index parent: {error}"))?;
                if identity(&parent_before) != identity(&parent_after) {
                    return Err(format!(
                        "host_link_parent_drift_during_observation: {}",
                        path.display()
                    ));
                }
                return Ok(None);
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect host index {}: {error}",
                    path.display()
                ))
            }
        };
        if FileType::from_raw_mode(before.st_mode) != FileType::Symlink {
            return Err(format!(
                "host index conflict is not a symlink: {}",
                path.display()
            ));
        }
        #[cfg(test)]
        AFTER_LINK_NAMED_STAT_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        let target = rustix::fs::readlinkat(&parent_fd, name, Vec::new())
            .map_err(|error| format!("cannot read link {}: {error}", path.display()))?
            .into_bytes();
        if target.len() > 16 * 1024 {
            return Err(format!(
                "host link target exceeds byte budget: {}",
                path.display()
            ));
        }
        let after = rustix::fs::statat(&parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("cannot revalidate host index {}: {error}", path.display()))?;
        let parent_after = rustix::fs::fstat(&parent_fd)
            .map_err(|error| format!("cannot revalidate host index parent: {error}"))?;
        if binding(&before) != binding(&after)
            || identity(&parent_before) != identity(&parent_after)
        {
            return Err(format!(
                "host_link_target_drift_during_observation: {}",
                path.display()
            ));
        }
        Ok(Some(target))
    }

    pub(super) fn binding(stat: &Stat) -> (u64, u64, u32, i128, i128, i128, i128, i128) {
        (
            stat.st_dev as u64,
            stat.st_ino,
            stat.st_mode as u32,
            stat.st_size as i128,
            stat.st_mtime as i128,
            stat.st_mtime_nsec as i128,
            stat.st_ctime as i128,
            stat.st_ctime_nsec as i128,
        )
    }

    fn ensure_same(before: &Stat, after: &Stat, label: &str) -> Result<(), String> {
        if binding(before) == binding(after) {
            Ok(())
        } else {
            Err(format!("{label} identity drift during observation"))
        }
    }

    fn ensure_same_identity(before: &Stat, after: &Stat, label: &str) -> Result<(), String> {
        if identity(before) == identity(after) {
            Ok(())
        } else {
            Err(format!("{label} identity drift during observation"))
        }
    }

    fn identity(stat: &Stat) -> (u64, u64, u32) {
        (stat.st_dev as u64, stat.st_ino, stat.st_mode as u32)
    }

    pub(super) fn mode(stat: &Stat) -> u32 {
        u32::from(stat.st_mode) & 0o7777
    }
}

pub(crate) fn observe_skill_source(
    path: &Path,
    policy: SourcePolicy,
) -> Result<SourceObservation, String> {
    #[cfg(unix)]
    {
        unix::observe(path, policy)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, policy);
        Err("descriptor_semantics_unavailable_for_skill_source".to_string())
    }
}

#[cfg(unix)]
pub(crate) fn observe_skill_source_at(
    root: &DescriptorRoot,
    relative_path: &Path,
    policy: SourcePolicy,
) -> Result<SourceObservation, String> {
    unix::observe_directory_at(root, relative_path, policy)
}

pub(crate) fn observe_link_target(path: &Path) -> Result<Option<Vec<u8>>, String> {
    #[cfg(unix)]
    {
        unix::observe_link(path)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("descriptor_semantics_unavailable_for_skill_materialization".to_string())
    }
}

pub(crate) fn observe_bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
    label: &str,
) -> Result<BoundedRegularFileObservation, String> {
    #[cfg(unix)]
    {
        unix::observe_bounded_file(path, maximum_bytes, label)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, maximum_bytes, label);
        Err("descriptor_semantics_unavailable_for_bounded_regular_file".to_string())
    }
}

#[cfg(unix)]
pub(crate) fn observe_bounded_regular_file_at(
    root: &DescriptorRoot,
    relative_path: &Path,
    maximum_bytes: u64,
    label: &str,
) -> Result<BoundedRegularFileObservation, String> {
    match unix::observe_optional_bounded_file_at(root, relative_path, maximum_bytes, label)? {
        OptionalBoundedRegularFileObservation::Present(observed) => Ok(observed),
        OptionalBoundedRegularFileObservation::Absent(_) => Err(format!("{label}_not_found")),
    }
}

#[cfg(unix)]
pub(crate) fn observe_optional_bounded_regular_file_at(
    root: &DescriptorRoot,
    relative_path: &Path,
    maximum_bytes: u64,
    label: &str,
) -> Result<OptionalBoundedRegularFileObservation, String> {
    unix::observe_optional_bounded_file_at(root, relative_path, maximum_bytes, label)
}

#[cfg(all(test, unix))]
mod descriptor_root_tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn stable_intermediate_symlink_resolves_to_a_held_physical_root() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let physical = base.join("physical/root");
        std::fs::create_dir_all(&physical).unwrap();
        symlink("physical", base.join("logical")).unwrap();

        let root = DescriptorRoot::open_absolute(&base.join("logical/root"), "test root").unwrap();
        assert_eq!(root.physical_path(), physical);
        root.revalidate("test root").unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_var_logical_path_resolves_without_canonicalize_authority() {
        let root =
            DescriptorRoot::open_absolute(Path::new("/var/tmp"), "macOS logical root").unwrap();
        assert_eq!(root.physical_path(), Path::new("/private/var/tmp"));
    }

    #[test]
    fn intermediate_symlink_swap_is_rejected_before_following_outside() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let inside = base.join("inside/root");
        let outside = base.join("outside/root");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside untouched").unwrap();
        let link = base.join("logical");
        symlink("inside", &link).unwrap();
        let hook_ran = Arc::new(AtomicBool::new(false));
        let hook_ran_for_hook = Arc::clone(&hook_ran);
        let link_for_hook = link.clone();
        set_after_descriptor_symlink_named_stat_hook(Box::new(move || {
            std::fs::remove_file(&link_for_hook).unwrap();
            symlink("outside", &link_for_hook).unwrap();
            hook_ran_for_hook.store(true, Ordering::SeqCst);
        }));

        let error = DescriptorRoot::open_absolute(&base.join("logical/root"), "test root")
            .err()
            .expect("symlink swap must fail closed");
        assert!(hook_ran.load(Ordering::SeqCst));
        assert!(
            error.contains("symlink") && error.contains("drift"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside untouched"
        );
    }

    #[test]
    fn symlink_loop_and_depth_are_bounded() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        symlink("b", base.join("a")).unwrap();
        symlink("a", base.join("b")).unwrap();
        let error = DescriptorRoot::open_absolute(&base.join("a"), "test root")
            .err()
            .expect("symlink loop must be bounded");
        assert!(error.contains("symlink depth"), "{error}");
    }

    #[test]
    fn requested_parent_traversal_is_rejected_lexically() {
        let error = DescriptorRoot::open_absolute(Path::new("/tmp/../etc"), "test root")
            .err()
            .expect("requested traversal must fail");
        assert!(error.contains("non-normal path component"), "{error}");
    }

    #[test]
    fn symlink_target_cannot_traverse_above_filesystem_root() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        symlink("../".repeat(64), base.join("escape")).unwrap();

        let error = DescriptorRoot::open_absolute(&base.join("escape"), "test root")
            .err()
            .expect("symlink traversal above root must fail");
        assert!(error.contains("escapes filesystem root"), "{error}");
    }
}
