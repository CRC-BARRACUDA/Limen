//! Finding modules on disk.

use std::path::PathBuf;
use limen_core::engine::*;
fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("limen-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_module(root: &std::path::Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("limen.toml"),
        format!(
            "[module]\nname = \"{name}\"\nversion = \"0.1.0\"\nlanguage = \"python\"\nentry = \"main.py\"\n"
        ),
    )
    .unwrap();
}

#[test]
fn discovers_module_subdirs_and_skips_missing_roots() {
    let root = tmpdir("root");
    write_module(&root, "alpha");
    write_module(&root, "beta");

    let dirs = collect_module_dirs(&[root.clone(), PathBuf::from("/no/such/dir")]).unwrap();
    assert_eq!(dirs.len(), 2);
    assert!(dirs.iter().all(|d| d.join("limen.toml").is_file()));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn treats_a_root_that_is_itself_a_module_as_one_module() {
    let root = tmpdir("single");
    // The root itself holds the manifest.
    std::fs::write(
        root.join("limen.toml"),
        "[module]\nname = \"solo\"\nversion = \"0.1.0\"\nlanguage = \"lua\"\nentry = \"m.lua\"\n",
    )
    .unwrap();

    let dirs = collect_module_dirs(std::slice::from_ref(&root)).unwrap();
    assert_eq!(dirs, vec![root.clone()]);

    std::fs::remove_dir_all(&root).ok();
}
