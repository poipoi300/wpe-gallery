use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_wpe-gallery")
}
fn run(home: &Path, config: &Path, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config)
        .env("WPE_OUTPUT", "TEST-1")
        .env("WPE_BG", "1001")
        .env("WPE_FPS", "240")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("NIRI_SOCKET")
        .env_remove("DISPLAY")
        .output()
        .unwrap()
}
fn fixture(home: &Path, id: &str) -> std::path::PathBuf {
    let path = home
        .join(".local/share/Steam/steamapps/workshop/content/431960")
        .join(id);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("project.json"), b"{}").unwrap();
    path
}
#[test]
fn launcher_requires_review_rechecks_fallback_and_passes_verified_paths() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    let one = fixture(&home, "1001");
    let two = fixture(&home, "1002");
    let whitelist = config.join("wpe-gallery/whitelist.json");
    fs::create_dir_all(whitelist.parent().unwrap()).unwrap();
    assert!(!run(&home, &config, &["--renderer-args"]).status.success());
    let hash = wpe_gallery::safety::content_hash(&one).unwrap();
    let review = json!({"verdict":"safe","reviewed":"2026-11-16","reviewer":"synthetic test","notes":"","content_hash":hash});
    fs::write(
        &whitelist,
        serde_json::to_vec(&json!({"1001":review})).unwrap(),
    )
    .unwrap();
    let result = run(&home, &config, &["--renderer-args"]);
    assert!(result.status.success(), "{:?}", result);
    let args: Vec<String> = serde_json::from_slice(&result.stdout).unwrap();
    assert!(args.contains(&one.to_str().unwrap().to_string()));
    assert!(args.contains(&"240".into()));
    let pending = config.join("linux-wallpaperengine/assignments.json");
    fs::create_dir_all(pending.parent().unwrap()).unwrap();
    fs::write(
        &pending,
        br#"{"version":1,"outputs":{"TEST-1":{"wallpaper_id":"1002"}}}"#,
    )
    .unwrap();
    fs::write(
        pending.with_file_name("last-safe-assignments.json"),
        br#"{"version":1,"outputs":{"TEST-1":{"wallpaper_id":"1001"}}}"#,
    )
    .unwrap();
    let result = run(&home, &config, &["--renderer-args"]);
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("falling back to last safe"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains(two.to_str().unwrap()));
    fs::write(one.join("new-file"), b"update").unwrap();
    assert!(!run(&home, &config, &["--renderer-args"]).status.success());
    let mut reviews: Value = serde_json::from_slice(&fs::read(&whitelist).unwrap()).unwrap();
    reviews["1001"]["verdict"] = json!("blocked");
    fs::write(&whitelist, serde_json::to_vec(&reviews).unwrap()).unwrap();
    assert!(!run(&home, &config, &["--renderer-args"]).status.success());
    fs::write(&whitelist, b"{broken").unwrap();
    assert!(!run(&home, &config, &["--renderer-args"]).status.success());
}
#[test]
fn matches_independent_wpe_gallery_content_v1_golden() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a"), b"one").unwrap();
    let mut h = Sha256::new();
    h.update(b"wpe-gallery-content-v1\0");
    h.update(b"F");
    h.update(1u64.to_be_bytes());
    h.update(b"a");
    h.update(3u64.to_be_bytes());
    h.update(b"one");
    assert_eq!(
        wpe_gallery::safety::content_hash(temp.path()).unwrap(),
        format!("{:x}", h.finalize())
    );
}
