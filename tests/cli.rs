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
fn schema_is_discoverable_and_marks_apply_as_mutating() {
    let out = run(&["schema"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["clispec"], "0.3");
    assert!(
        v["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "apply" && c["mutating"] == true)
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
fn missing_command_and_apply_without_approval_are_usage_errors() {
    for args in [&[][..], &["apply", "/path/does/not/exist"][..]] {
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
#[test]
fn balance_and_dry_run_work_offline() {
    let path = std::env::temp_dir().join(format!("yoobi-cli-test-{}.json", std::process::id()));
    std::fs::write(&path,r#"{"version":1,"base_url":"https://tenant.yoobi.nl","employee_id":"employee","month":"2026-11","project_id":"activity","opening_balance_minutes":450,"days":[{"date":"2026-11-06","actual_minutes":345,"before_minutes":0,"hours":8}]}"#).unwrap();
    let out = run(&["balance", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["balance_minutes"], 315);
    let out = run(&["apply", path.to_str().unwrap(), "--dry-run"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["dry_run"], true);
    let _ = std::fs::remove_file(path);
}

#[test]
fn text_output_is_a_real_ledger_and_handles_extreme_debt() {
    let path = std::env::temp_dir().join(format!("yoobi-text-{}.json", std::process::id()));
    std::fs::write(
        &path,
        r#"{"opening_balance_minutes":-9223372036854775808,"days":[]}"#,
    )
    .unwrap();
    let out = run(&["balance", path.to_str().unwrap(), "--output", "text"]);
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.contains("debt"));
    assert!(s.contains("Opening balance"));
    assert!(!s.starts_with('{'));
    let out = run(&["--output", "text", "balance", path.to_str().unwrap()]);
    assert!(out.status.success());
    std::fs::remove_file(path).unwrap();
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
