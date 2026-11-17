//! CLI-level tests for the honest hook warning and the refusal of unsafe
//! scripts (audit M6).  These drive the real `dmux` binary so they observe the
//! actual stderr a user sees.

use std::path::PathBuf;
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dmux-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn hook_command_prints_the_full_permissions_warning() {
    let dir = temp_dir("warn");
    let results = dir.join("results.json");
    std::fs::write(&results, "{}").unwrap();
    let script = dir.join("hook.py");
    std::fs::write(
        &script,
        "import json, sys\njson.dump({'findings': []}, sys.stdout)\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .args(["hook", results.to_str().unwrap(), script.to_str().unwrap()])
        .output()
        .expect("run dmux hook");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("full permissions"),
        "the honest hook warning is missing from stderr: {stderr}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn hook_command_refuses_a_world_writable_script() {
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_dir("writable");
    let results = dir.join("results.json");
    std::fs::write(&results, "{}").unwrap();
    let script = dir.join("hook.py");
    std::fs::write(&script, "import sys\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o666)).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dmux"))
        .args(["hook", results.to_str().unwrap(), script.to_str().unwrap()])
        .output()
        .expect("run dmux hook");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("group- or world-writable"),
        "expected the world-writable script to be refused, got: {stderr}"
    );
    assert!(!out.status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}
