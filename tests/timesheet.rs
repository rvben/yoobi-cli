use yoobi_cli::timesheet::{days_in_month, validate_date, validate_month};

#[test]
fn dates_follow_the_calendar_including_century_leap_years() {
    assert_eq!(days_in_month("2000-02").unwrap(), 29);
    assert_eq!(days_in_month("2100-02").unwrap(), 28);
    assert!(validate_date("2028-02-29").is_ok());
    for date in ["2026-02-29", "2026-04-31", "2026-11-00", "2026-11-31"] {
        assert!(validate_date(date).is_err(), "{date}");
    }
}

#[test]
fn malformed_and_unicode_dates_fail_without_panicking() {
    for month in [
        "",
        "0000-01",
        "2026-00",
        "2026-13",
        "2026-1",
        "２０２６-１１",
    ] {
        assert!(validate_month(month).is_err(), "{month}");
    }
    for date in ["", "2026-11", "２０２６-11-01", "2026-11-🦀", "2026-11-1"] {
        assert!(validate_date(date).is_err(), "{date}");
    }
}
