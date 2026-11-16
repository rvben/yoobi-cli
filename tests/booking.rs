use yoobi_cli::{
    booking::{BookingEdit, parse_duration},
    tui::demo,
};
#[test]
fn durations_are_exact_and_explicit() {
    for (input, want) in [
        ("7h17", 437),
        ("7:17", 437),
        ("437m", 437),
        ("0m", 0),
        ("24h00", 1440),
        ("1h", 60),
        (" 1m ", 1),
    ] {
        assert_eq!(parse_duration(input).unwrap(), want, "{input}");
    }
    for input in [
        "8",
        "7.5",
        "7h60",
        "7:1",
        "1:60",
        "24h01",
        "1441m",
        "-1m",
        "+1m",
        "1e2m",
        "1h2h",
        "０m",
        "999999999999999999999h",
    ] {
        assert!(parse_duration(input).is_err(), "{input}");
    }
}
#[test]
fn previews_bind_identity_and_validate_dates_and_duration() {
    let sheet = demo("2026-11");
    let e = BookingEdit::from_sheet(&sheet, "demo-delivery", "2026-11-11", 437).unwrap();
    assert_eq!(e.minutes, 437);
    assert_eq!(e.base_url, sheet.base_url);
    assert_eq!(e.employee_id, sheet.employee_id);
    for (activity, date, m) in [
        ("missing", "2026-11-11", 437),
        ("demo-delivery", "2026-12-01", 437),
        ("demo-delivery", "2026-11-31", 437),
        ("demo-delivery", "2026-11-11", 1441),
        ("demo-delivery", "2026-11-02", 1),
    ] {
        assert!(BookingEdit::from_sheet(&sheet, activity, date, m).is_err());
    }
    let mut bad = e;
    bad.date = "x".into();
    assert!(bad.validate().is_err());
}
