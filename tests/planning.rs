use std::collections::{BTreeMap, BTreeSet};
use yoobi_cli::plan::{
    Ledger, LedgerDay, Plan, Project, Timesheet, WorkDay, WorkInput, validate_date, validate_month,
};
fn sheet() -> Timesheet {
    Timesheet {
        base_url: "https://tenant.yoobi.nl".into(),
        employee_id: "employee".into(),
        month: "2026-11".into(),
        total_minutes: 0,
        projects: vec![Project {
            id: "activity".into(),
            name: "Project / Activity".into(),
            days: BTreeMap::new(),
            locked_days: BTreeSet::new(),
        }],
    }
}
fn input(minutes: u32, hours: Option<u8>) -> WorkInput {
    WorkInput {
        month: "2026-11".into(),
        project_id: "activity".into(),
        opening_balance_minutes: 450,
        days: vec![WorkDay {
            date: "2026-11-06".into(),
            actual_minutes: minutes,
            hours,
        }],
    }
}
#[test]
fn six_hour_boundary_rounds_up_and_explicit_override_is_preserved() {
    assert_eq!(
        Plan::build(input(359, None), &sheet()).unwrap().days[0].hours,
        4
    );
    assert_eq!(
        Plan::build(input(360, None), &sheet()).unwrap().days[0].hours,
        8
    );
    assert_eq!(
        Plan::build(input(345, Some(8)), &sheet()).unwrap().days[0].hours,
        8
    );
}
#[test]
fn approved_november_plan_leaves_three_hours_forty_five_surplus() {
    let dates = ["02", "03", "04", "05", "06", "09", "10", "11", "12", "13"];
    let actual = [525, 495, 525, 360, 345, 510, 495, 240, 465, 375];
    let mut work = input(0, None);
    work.days = dates
        .into_iter()
        .zip(actual)
        .map(|(day, actual_minutes)| WorkDay {
            date: format!("2026-11-{day}"),
            actual_minutes,
            hours: Some(if day == "11" { 4 } else { 8 }),
        })
        .collect();
    let plan = Plan::build(work, &sheet()).unwrap();
    let balance = plan.balance().unwrap();
    assert_eq!(balance.actual_minutes, 4335);
    assert_eq!(balance.booked_minutes, 4560);
    assert_eq!(balance.balance_minutes, 225);
}
#[test]
fn locked_changes_refused_but_unchanged_locked_days_are_valid() {
    let mut s = sheet();
    s.projects[0].locked_days.insert("2026-11-06".into());
    assert_eq!(
        Plan::build(input(480, None), &s).unwrap_err().kind(),
        "conflict"
    );
    s.projects[0].days.insert("2026-11-06".into(), 480);
    assert!(Plan::build(input(480, None), &s).is_ok());
}
#[test]
fn plan_keeps_existing_hours_as_precondition() {
    let mut s = sheet();
    s.projects[0].days.insert("2026-11-06".into(), 240);
    let p = Plan::build(input(480, None), &s).unwrap();
    assert_eq!(p.days[0].before_minutes, 240);
    assert_eq!(p.employee_id, "employee");
}
#[test]
fn duplicate_dates_unknown_project_and_wrong_month_are_rejected() {
    let mut i = input(480, None);
    i.days.push(i.days[0].clone());
    assert!(Plan::build(i, &sheet()).is_err());
    let mut i = input(480, None);
    i.project_id = "unknown".into();
    assert!(Plan::build(i, &sheet()).is_err());
    let mut i = input(480, None);
    i.days[0].date = "2026-10-06".into();
    assert!(Plan::build(i, &sheet()).is_err());
}
#[test]
fn invalid_duration_zero_work_and_unsupported_hours_are_refused() {
    for i in [input(1441, None), input(0, None), input(480, Some(6))] {
        assert!(Plan::build(i, &sheet()).is_err());
    }
}
#[test]
fn actual_calendar_days_and_unicode_are_validated_without_panics() {
    for month in ["2026-00", "2026-13", "2026-1", "0000-11", "éé-11"] {
        assert!(validate_month(month).is_err());
    }
    for date in ["2026-02-29", "2026-04-31", "2026-11-00", "éééé12"] {
        assert!(validate_date(date).is_err());
    }
    assert!(validate_date("2024-02-29").is_ok());
    assert!(validate_date("2000-02-29").is_ok());
    assert!(validate_date("1900-02-29").is_err());
}
#[test]
fn balance_spans_months_and_preserves_debt_and_minute_precision() {
    let ledger = Ledger {
        opening_balance_minutes: 0,
        days: vec![
            LedgerDay {
                date: "2026-10-30".into(),
                actual_minutes: 435,
                booked_minutes: 480,
            },
            LedgerDay {
                date: "2026-11-02".into(),
                actual_minutes: 525,
                booked_minutes: 480,
            },
        ],
    };
    assert_eq!(ledger.balance().unwrap().balance_minutes, 0);
    let debt = Ledger {
        opening_balance_minutes: -1,
        days: vec![],
    };
    assert_eq!(debt.balance().unwrap().balance_minutes, -1);
    let overflow = Ledger {
        opening_balance_minutes: i64::MAX,
        days: vec![LedgerDay {
            date: "2026-11-02".into(),
            actual_minutes: 1,
            booked_minutes: 0,
        }],
    };
    assert!(overflow.balance().is_err());
}
#[test]
fn unknown_json_fields_are_not_silently_ignored() {
    assert!(
        serde_json::from_str::<WorkInput>(
            r#"{"month":"2026-11","project_id":"x","days":[],"hour":8}"#
        )
        .is_err()
    );
}
