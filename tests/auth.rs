use mockito::Server;
use std::{
    io::Write,
    process::{Command, Stdio},
};
const BIN: &str = env!("CARGO_BIN_EXE_yoobi-cli");
fn isolated(dir: &std::path::Path) -> Command {
    let mut c = Command::new(BIN);
    c.env("YOOBI_CONFIG_DIR", dir)
        .env_remove("YOOBI_BASE_URL")
        .env_remove("YOOBI_USERNAME")
        .env_remove("YOOBI_PASSWORD")
        .env_remove("YOOBI_PASSWORD_FILE");
    c
}
#[test]
fn login_refuses_missing_noninteractive_credentials_and_logout_is_local() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("profile");
    let o = isolated(&missing).args(["auth", "login"]).output().unwrap();
    assert_eq!(o.status.code(), Some(6));
    assert!(o.stdout.is_empty());
    assert!(!missing.exists());
    let o = isolated(&missing)
        .args(["auth", "logout"])
        .output()
        .unwrap();
    assert!(o.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&o.stdout).unwrap()["removed"],
        false
    );
    assert!(!missing.exists());
}
#[test]
fn failed_login_verification_never_creates_a_saved_login_or_leaks_the_password() {
    let mut server = Server::new();
    server
        .mock("POST", "/security/doLogin")
        .with_body("login denied")
        .create();
    server
        .mock("GET", "/timesheet/index")
        .with_body("sign in")
        .create();
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("profile");
    let mut child = isolated(&missing)
        .args([
            "auth",
            "login",
            "--base-url",
            &server.url(),
            "--username",
            "user",
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"PRIVATE_PASSWORD\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(out.stdout.is_empty());
    assert!(!missing.exists());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("PRIVATE_PASSWORD"));
}
#[test]
fn local_status_does_not_open_the_keychain_for_a_saved_profile() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("login.json"),r#"{"version":1,"base_url":"https://example.invalid","username":"private-user","credential_id":"00000000-0000-4000-8000-000000000001"}"#).unwrap();
    let out = isolated(dir.path())
        .args(["auth", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["configured"], true);
    assert_eq!(v["source"], "keychain");
    assert_eq!(v["verified"], false);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("private-user"));
}
#[test]
fn password_stdin_conflicts_are_refused_before_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let out = isolated(dir.path())
        .args([
            "auth",
            "login",
            "--base-url",
            "https://example.invalid",
            "--username",
            "u",
            "--password-stdin",
        ])
        .env("YOOBI_PASSWORD", "PRIVATE_PASSWORD")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("PRIVATE_PASSWORD"));
}
#[test]
fn complete_environment_credentials_override_even_invalid_saved_metadata() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("login.json"), "broken metadata").unwrap();
    let out = isolated(dir.path())
        .args(["auth", "status"])
        .env("YOOBI_BASE_URL", "https://example.invalid")
        .env("YOOBI_USERNAME", "user")
        .env("YOOBI_PASSWORD", "PRIVATE_PASSWORD")
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["configured"], true);
    assert_eq!(v["source"], "environment");
}
