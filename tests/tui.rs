use std::process::Command;
const BIN: &str = env!("CARGO_BIN_EXE_yoobi-cli");
fn run(args: &[&str]) -> std::process::Output {
    Command::new(BIN)
        .args(args)
        .env_remove("YOOBI_BASE_URL")
        .env_remove("YOOBI_USERNAME")
        .env_remove("YOOBI_PASSWORD")
        .env_remove("YOOBI_PASSWORD_FILE")
        .output()
        .unwrap()
}
#[test]
fn refuses_piped_input_before_auth_or_file_access() {
    let o = run(&["tui"]);
    assert_eq!(o.status.code(), Some(6));
    let e: serde_json::Value = serde_json::from_slice(&o.stderr).unwrap();
    assert_eq!(e["error"]["kind"], "tty_required");
    assert!(o.stdout.is_empty());
}
#[test]
fn demo_snapshot_is_offline_and_clearly_labelled() {
    let o = run(&["tui", "--demo", "--snapshot", "--month", "2026-11"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let s = String::from_utf8(o.stdout).unwrap();
    assert!(s.contains("DEMO"));
    assert!(s.contains("Consultancy / Delivery"));
    assert!(s.contains("locked"));
    assert!(s.contains("Booked"));
    assert!(!s.contains("Compare with actual work"));
    assert!(!s.contains("Projected balance"));
    assert!(!s.contains("Actual"));
    assert!(!s.contains('\x1b'));
    assert!(o.stderr.is_empty());
}
#[test]
fn capture_is_bounded_and_never_reads_connected_data() {
    for args in [
        vec!["tui", "--snapshot"],
        vec!["tui", "--demo", "--snapshot", "--width", "1000"],
        vec!["tui", "--demo", "--snapshot", "--output", "json"],
        vec!["tui", "--demo", "--snapshot", "--month", "2026-13"],
    ] {
        let o = run(&args);
        assert!(!o.status.success());
        assert!(o.stdout.is_empty());
        assert!(serde_json::from_slice::<serde_json::Value>(&o.stderr).is_ok());
    }
}
#[test]
fn narrow_layout_preserves_quit_and_navigation() {
    let o = run(&[
        "tui",
        "--demo",
        "--snapshot",
        "--width",
        "40",
        "--height",
        "16",
    ]);
    assert!(o.status.success());
    let s = String::from_utf8(o.stdout).unwrap();
    assert!(s.contains("q quit"));
    assert!(s.contains("DATE"));
}
#[test]
fn schema_declares_explicitly_confirmed_interactive_writes() {
    let o = run(&["schema"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let t = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "tui")
        .unwrap();
    assert_eq!(t["effects"], "idempotent");
    assert_eq!(t["mutating"], true);
    assert_eq!(t["extensions"]["requires_explicit_save"], true);
    assert_eq!(t["extensions"]["demo_is_offline"], true);
    assert_eq!(t["requires_tty"], true);
    assert_eq!(t["output_kind"], "opaque");
}
