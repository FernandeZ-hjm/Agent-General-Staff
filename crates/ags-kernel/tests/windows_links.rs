#![cfg(windows)]

use std::fs;

#[test]
fn managed_directory_link_install_repair_and_remove_lifecycle() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let source = home.path().join("source/demo");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo need\n---\n",
    )
    .unwrap();

    ags_kernel::sync::install_skill_body("demo", &source).unwrap();
    let link = home.path().join(".agents/skills/demo");
    assert!(junction::exists(&link).unwrap());
    assert!(link.join("SKILL.md").is_file());

    fs::remove_dir_all(&source).unwrap();
    assert!(!link.exists(), "junction must now be dangling");
    let replacement = home.path().join("replacement/demo");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("SKILL.md"),
        "---\nname: demo\ndescription: Demo.\ntriggers:\n  - demo need\n---\n",
    )
    .unwrap();
    ags_kernel::sync::install_skill_body("demo", &replacement).unwrap();
    assert!(junction::exists(&link).unwrap());
    assert_eq!(
        junction::get_target(&link).unwrap(),
        replacement.canonicalize().unwrap()
    );

    assert!(ags_kernel::sync::remove_skill_body("demo").unwrap());
    assert!(
        fs::symlink_metadata(&link).is_err(),
        "junction delete must not leave an empty directory behind"
    );

    fs::create_dir_all(&link).unwrap();
    let error = ags_kernel::sync::remove_skill_body("demo").unwrap_err();
    assert_eq!(error.code, "skill_body_not_managed_symlink");
    assert!(link.is_dir(), "unmanaged directory must remain untouched");
}
