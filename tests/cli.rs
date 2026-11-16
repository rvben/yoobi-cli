use std::process::Command;
const BIN: &str = env!("CARGO_BIN_EXE_yoobi-cli");
fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env_remove("YOOBI_PASSWORD")
        .env_remove("YOOBI_USERNAME")
        .env_remove("YOOBI_BASE_URL")
        .env_remove("YOOBI_PASSWORD_FILE")
        .output()
        .unwrap()
}
fn error(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap()["error"]
        .clone()
}
#[test]
fn schema_is_discoverable_and_timesheet_commands_are_read_only() {
    let out = run(&["schema"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["clispec"], "0.3");
    assert!(
        v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| matches!(c["name"].as_str(), Some("timesheet" | "projects" | "tui")))
            .all(|c| c["effects"] == "read_only")
    );
}
#[test]
fn help_and_completions_work_without_authentication() {
    for args in [&["--help"][..], &["completions", "zsh"][..]] {
        let out = run(args);
        assert!(out.status.success());
        assert!(!out.stdout.is_empty());
    }
}
#[test]
fn missing_and_removed_commands_are_usage_errors() {
    for args in [
        &[][..],
        &["apply"][..],
        &["plan"][..],
        &["balance"][..],
        &["tui", "--plan", "unused.json"][..],
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(error(&out)["kind"], "usage");
    }
}
#[test]
fn malformed_month_is_refused_before_authentication() {
    let out = run(&["timesheet", "--month", "2026-13"]);
    assert_eq!(error(&out)["kind"], "invalid_input");
}
#[test]
fn auth_status_is_local_and_does_not_print_credentials() {
    let out = Command::new(BIN)
        .args(["auth", "status"])
        .env("YOOBI_PASSWORD", "SECRET_NEVER_PRINT")
        .env("YOOBI_USERNAME", "PRIVATE_USER")
        .env("YOOBI_BASE_URL", "https://tenant.yoobi.nl")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(!stdout.contains("SECRET_NEVER_PRINT"));
    assert!(!stdout.contains("PRIVATE_USER"));
}
#[cfg(unix)]
#[test]
fn insecure_password_files_are_rejected_without_leaking_contents() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("yoobi-insecure-password-{}", std::process::id()));
    std::fs::write(&path, "NEVER_PRINT_THIS_PASSWORD").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let out = Command::new(BIN)
        .args(["timesheet", "--month", "2026-11"])
        .env("YOOBI_BASE_URL", "https://example.invalid")
        .env("YOOBI_USERNAME", "user")
        .env_remove("YOOBI_PASSWORD")
        .env("YOOBI_PASSWORD_FILE", &path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("NEVER_PRINT_THIS_PASSWORD"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("chmod 600"));
    std::fs::remove_file(path).unwrap();
}
