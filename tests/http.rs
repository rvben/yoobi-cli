use mockito::{Matcher, Server};
use serde_json::json;
use yoobi_cli::{api::Client, config::Config};

fn grid(hours: u8, locked: bool) -> String {
    let mut decl = vec![json!({"d":1})];
    let mut cells = String::new();
    for d in 1..=30 {
        let date = format!("2026-11-{d:02}");
        decl.push(json!({"dtdate":date,"time":if d==2 {hours} else {0},"timeMsg":"","ref":""}));
        cells.push_str(&format!(
            "<div class=\"cell {} {{date:'{date}', p:1,a:1,d:{d}}}\"></div>",
            if locked { "approved" } else { "" }
        ));
    }
    let data = json!({"monthTotal":hours,"projects":[{}, {"pID":"project","activities":[{}, {"aID":"activity","allowOvertime":0,"defaultOvertimeID":"","decl":decl}]}]});
    format!(
        "<div class='project' id='project_1-l'><span class='fullName'>Example</span></div><div id='activity_1.1' title='Work'></div><div class=\"activity {{type:'time',alttype:[],bNoteRequired:false}}\" id='activity_1.1-d'>{cells}</div><div class='layoutRightBottom'><span class='daylytotal'>{hours}</span></div><div id='stData'>{}</div>",
        data.to_string()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    )
}
fn login(server: &mut Server) -> Client {
    server
        .mock("POST", "/security/doLogin")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("username".into(), "user".into()),
            Matcher::UrlEncoded("pass".into(), "secret".into()),
        ]))
        .with_status(200)
        .with_body("ok")
        .create();
    server
        .mock("GET", "/timesheet/index")
        .with_body("<input id='employeeID' value='employee'>")
        .create();
    Client::new(&Config::new(&server.url(), "user".into(), "secret".into()).unwrap()).unwrap()
}
#[test]
fn reads_month_totals_over_one_day() {
    let mut s = Server::new();
    let c = login(&mut s);
    // Independent daily entries, month aggregate exceeds the 24-hour daily limit.
    let html = grid(8, false)
        .replace("\"monthTotal\":8", "\"monthTotal\":32")
        .replace("daylytotal'>8", "daylytotal'>32");
    let html = html.replace(
        "\"dtdate\":\"2026-11-03\",\"ref\":\"\",\"time\":0",
        "\"dtdate\":\"2026-11-03\",\"ref\":\"\",\"time\":24",
    );
    s.mock("POST", "/timesheet/timesheet")
        .with_body(html)
        .create();
    assert_eq!(c.timesheet("2026-11").unwrap().total_minutes, 1920);
}
#[test]
fn totals_must_agree() {
    let mut s = Server::new();
    let c = login(&mut s);
    s.mock("POST", "/timesheet/timesheet")
        .with_body(grid(8, false).replace("daylytotal'>8", "daylytotal'>4"))
        .create();
    assert!(c.timesheet("2026-11").is_err());
}
#[test]
fn rejects_unsafe_origins_and_redacts_credentials() {
    for url in [
        "http://example.com",
        "https://user:pass@example.com",
        "https://example.com/path",
    ] {
        assert!(Config::new(url, "u".into(), "p".into()).is_err());
    }
    let c = Config::new(
        "https://example.com/",
        "private-user".into(),
        "private-password".into(),
    )
    .unwrap();
    let debug = format!("{c:?}");
    assert!(!debug.contains("private-user"));
    assert!(!debug.contains("private-password"));
}

#[test]
fn rejected_login_and_expired_session_are_auth_errors() {
    let mut s = Server::new();
    s.mock("POST", "/security/doLogin")
        .with_status(200)
        .with_body("<form>Sign in</form>")
        .create();
    s.mock("GET", "/timesheet/index")
        .with_body("<form>Sign in</form>")
        .create();
    let config = Config::new(&s.url(), "user".into(), "secret".into()).unwrap();
    assert!(matches!(
        Client::new(&config),
        Err(yoobi_cli::Error::Auth { .. })
    ));
    let mut s = Server::new();
    let c = login(&mut s);
    s.mock("POST", "/timesheet/timesheet")
        .with_status(403)
        .with_body("sensitive upstream details")
        .create();
    let e = c.timesheet("2026-11").unwrap_err();
    assert!(matches!(e, yoobi_cli::Error::Auth { .. }));
    assert!(!e.to_string().contains("sensitive"));
}
#[test]
fn multiple_activities_keep_separate_values_and_totals() {
    let mut s = Server::new();
    let c = login(&mut s);
    let original = grid(8, false);
    let (head, data) = original.split_once("<div id='stData'>").unwrap();
    let mut data: serde_json::Value =
        serde_json::from_str(data.strip_suffix("</div>").unwrap()).unwrap();
    let mut second = data["projects"][1]["activities"][1].clone();
    second["aID"] = json!("activity-two");
    second["decl"][2]["time"] = json!(4);
    data["projects"][1]["activities"]
        .as_array_mut()
        .unwrap()
        .push(second);
    data["monthTotal"] = json!(12);
    let row = head[head.find("<div class=\"activity").unwrap()..]
        .split("<div class='layoutRightBottom'>")
        .next()
        .unwrap()
        .replace("activity_1.1-d", "activity_1.2-d");
    let html = format!(
        "{}{row}<div id='stData'>{data}</div>",
        head.replace("daylytotal'>8", "daylytotal'>12")
    );
    s.mock("POST", "/timesheet/timesheet")
        .with_body(html)
        .create();
    let sheet = c.timesheet("2026-11").unwrap();
    assert_eq!(sheet.total_minutes, 720);
    assert_eq!(sheet.projects.len(), 2);
    assert_eq!(sheet.projects[0].days["2026-11-02"], 480);
    assert_eq!(sheet.projects[1].days["2026-11-02"], 240);
}

#[test]
fn reads_exact_minutes_and_does_not_treat_booking_requirements_as_locks() {
    let mut s = Server::new();
    let c = login(&mut s);
    let html = grid(8, false)
        .replace("\"time\":8", "\"time\":7.25")
        .replace("\"monthTotal\":8", "\"monthTotal\":7.25")
        .replace("daylytotal'>8", "daylytotal'>7.25")
        .replace("bNoteRequired:false", "bNoteRequired:true");
    s.mock("POST", "/timesheet/timesheet")
        .with_body(html)
        .create();
    let sheet = c.timesheet("2026-11").unwrap();
    assert_eq!(sheet.total_minutes, 435);
    assert_eq!(sheet.projects[0].days["2026-11-02"], 435);
    assert!(sheet.projects[0].locked_days.is_empty());
}

fn edited_grid(minutes: u32, locked: bool) -> String {
    let hours = f64::from(minutes) / 60.0;
    grid(8, locked)
        .replace("\"time\":8", &format!("\"time\":{hours}"))
        .replace("\"monthTotal\":8", &format!("\"monthTotal\":{hours}"))
        .replace("daylytotal'>8", &format!("daylytotal'>{hours}"))
        .replace("\"timeMsg\":\"\"", "\"timeMsg\":\"existing note\"")
        .replace("\"ref\":\"\"", "\"ref\":\"existing reference\"")
}
fn edit(s: &Server, minutes: u32) -> yoobi_cli::booking::BookingEdit {
    yoobi_cli::booking::BookingEdit {
        version: 1,
        base_url: s.url(),
        employee_id: "employee".into(),
        activity_id: "activity".into(),
        activity_name: "Example / Work".into(),
        date: "2026-11-02".into(),
        before_minutes: 480,
        minutes,
    }
}
#[test]
fn booking_preview_reads_without_writing() {
    let mut s = Server::new();
    let c = login(&mut s);
    let read = s
        .mock("POST", "/timesheet/timesheet")
        .with_body(edited_grid(480, false))
        .expect(1)
        .create();
    let write = s
        .mock("POST", "/declaration/saveDeclarations")
        .expect(0)
        .create();
    let preview = c.preview_booking("activity", "2026-11-02", 437).unwrap();
    assert_eq!(preview.minutes, 437);
    assert_eq!(preview.before_minutes, 480);
    read.assert();
    write.assert();
}
#[test]
fn saves_exact_minutes_preserves_notes_and_verifies_all_bookings() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let mut s = Server::new();
    let c = login(&mut s);
    let e = edit(&s, 437);
    let calls = Arc::new(AtomicUsize::new(0));
    let n = calls.clone();
    let read = s
        .mock("POST", "/timesheet/timesheet")
        .with_body_from_request(move |_| {
            edited_grid(
                if n.fetch_add(1, Ordering::SeqCst) == 0 {
                    480
                } else {
                    437
                },
                false,
            )
            .into_bytes()
        })
        .expect(2)
        .create();
    let diff = (437.0 - 480.0) / 60.0;
    let payload = json!([{"key":"activity-2026-11-02","value":[{"time":437.0/60.0,
        "timeMsg":"existing note","ref":"existing reference","p":1,"a":1,"d":2,"pID":"project",
        "timediff":diff,"timedifforiginal":diff,"bFromPlanning":false}]}]);
    let write = s
        .mock("POST", "/declaration/saveDeclarations")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("json".into(), payload.to_string()),
            Matcher::UrlEncoded("employeeID".into(), "employee".into()),
            Matcher::UrlEncoded("directSave".into(), "true".into()),
        ]))
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .expect(1)
        .create();
    let result = c.save_booking(&e).unwrap();
    assert!(result.verified && result.changed);
    assert_eq!(result.timesheet.total_minutes, 437);
    write.assert();
    read.assert();
}
#[test]
fn stale_locked_and_unsupported_bookings_never_save() {
    for html in [
        edited_grid(420, false),
        edited_grid(480, true),
        edited_grid(480, false).replace("bNoteRequired:false", "bNoteRequired:true"),
        edited_grid(480, false).replace("\"allowOvertime\":0", "\"allowOvertime\":1"),
        edited_grid(480, false).replace("\"timeMsg\":\"existing note\"", "\"timeMsg\":null"),
    ] {
        let mut s = Server::new();
        let c = login(&mut s);
        s.mock("POST", "/timesheet/timesheet")
            .with_body(html)
            .create();
        let write = s
            .mock("POST", "/declaration/saveDeclarations")
            .expect(0)
            .create();
        assert!(c.save_booking(&edit(&s, 437)).is_err());
        write.assert();
    }
}
#[test]
fn foreign_account_and_invalid_previews_fail_before_timesheet_reads() {
    let mut s = Server::new();
    let c = login(&mut s);
    let read = s.mock("POST", "/timesheet/timesheet").expect(0).create();
    let write = s
        .mock("POST", "/declaration/saveDeclarations")
        .expect(0)
        .create();
    let e = edit(&s, 437);
    for bad in [
        yoobi_cli::booking::BookingEdit {
            employee_id: "other".into(),
            ..e.clone()
        },
        yoobi_cli::booking::BookingEdit {
            base_url: "https://other.invalid".into(),
            ..e.clone()
        },
        yoobi_cli::booking::BookingEdit {
            minutes: 1441,
            ..e.clone()
        },
        yoobi_cli::booking::BookingEdit {
            date: "x".into(),
            ..e.clone()
        },
    ] {
        assert!(c.save_booking(&bad).is_err());
    }
    read.assert();
    write.assert();
}
#[test]
fn already_matching_locked_booking_is_an_idempotent_noop() {
    let mut s = Server::new();
    let c = login(&mut s);
    s.mock("POST", "/timesheet/timesheet")
        .with_body(edited_grid(437, true))
        .create();
    let write = s
        .mock("POST", "/declaration/saveDeclarations")
        .expect(0)
        .create();
    let r = c.save_booking(&edit(&s, 437)).unwrap();
    assert!(r.verified && !r.changed);
    write.assert();
}
#[test]
fn clearing_a_booking_writes_zero_then_verifies() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let mut s = Server::new();
    let c = login(&mut s);
    let n = Arc::new(AtomicUsize::new(0));
    s.mock("POST", "/timesheet/timesheet")
        .with_body_from_request(move |_| {
            edited_grid(
                if n.fetch_add(1, Ordering::SeqCst) == 0 {
                    480
                } else {
                    0
                },
                false,
            )
            .into_bytes()
        })
        .create();
    let w = s
        .mock("POST", "/declaration/saveDeclarations")
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .expect(1)
        .create();
    let r = c.save_booking(&edit(&s, 0)).unwrap();
    assert_eq!(r.timesheet.total_minutes, 0);
    w.assert();
}
#[test]
fn ambiguous_rejected_or_failed_save_is_never_retried() {
    for (status, body) in [
        (200, r#"{}"#),
        (200, r#"{"messages":[{"type":"error","text":"private"}]}"#),
        (503, "private"),
    ] {
        let mut s = Server::new();
        let c = login(&mut s);
        s.mock("POST", "/timesheet/timesheet")
            .with_body(edited_grid(480, false))
            .create();
        let w = s
            .mock("POST", "/declaration/saveDeclarations")
            .with_status(status)
            .with_body(body)
            .expect(1)
            .create();
        let error = c.save_booking(&edit(&s, 437)).unwrap_err();
        assert!(matches!(error, yoobi_cli::Error::SaveUncertain { .. }));
        assert!(!error.to_string().contains("private"));
        w.assert();
    }
}
#[test]
fn successful_ack_requires_readback_and_unchanged_other_days() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for changed_other_day in [false, true] {
        let mut s = Server::new();
        let c = login(&mut s);
        let n = Arc::new(AtomicUsize::new(0));
        s.mock("POST", "/timesheet/timesheet")
            .with_body_from_request(move |_| {
                if n.fetch_add(1, Ordering::SeqCst) == 0 || !changed_other_day {
                    return edited_grid(480, false).into_bytes();
                }
                edited_grid(437, false)
                    .replace(
                        "\"dtdate\":\"2026-11-03\",\"ref\":\"existing reference\",\"time\":0",
                        "\"dtdate\":\"2026-11-03\",\"ref\":\"existing reference\",\"time\":1",
                    )
                    .replace(
                        "\"monthTotal\":7.283333333333333",
                        "\"monthTotal\":8.283333333333333",
                    )
                    .replace(
                        "daylytotal'>7.283333333333333",
                        "daylytotal'>8.283333333333333",
                    )
                    .into_bytes()
            })
            .create();
        let w = s
            .mock("POST", "/declaration/saveDeclarations")
            .with_body(r#"{"messages":[{"type":"success"}]}"#)
            .expect(1)
            .create();
        assert!(matches!(
            c.save_booking(&edit(&s, 437)),
            Err(yoobi_cli::Error::SaveUncertain { .. })
        ));
        w.assert();
    }
}

#[test]
fn cli_previews_json_then_saves_only_with_explicit_approval() {
    use std::{
        process::Command,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    let mut s = Server::new();
    let _client = login(&mut s);
    let reads = Arc::new(AtomicUsize::new(0));
    let n = reads.clone();
    s.mock("POST", "/timesheet/timesheet")
        .with_body_from_request(move |_| {
            edited_grid(
                if n.fetch_add(1, Ordering::SeqCst) < 2 {
                    480
                } else {
                    437
                },
                false,
            )
            .into_bytes()
        })
        .expect(3)
        .create();
    let writes = s
        .mock("POST", "/declaration/saveDeclarations")
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .expect(1)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("booking.json");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yoobi-cli"))
            .args(args)
            .env("YOOBI_BASE_URL", s.url())
            .env("YOOBI_USERNAME", "user")
            .env("YOOBI_PASSWORD", "secret")
            .env_remove("YOOBI_PASSWORD_FILE")
            .output()
            .unwrap()
    };
    let preview = run(&[
        "booking",
        "preview",
        "--activity",
        "activity",
        "--date",
        "2026-11-02",
        "--duration",
        "7h17",
    ]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(v["minutes"], 437);
    std::fs::write(&path, preview.stdout).unwrap();
    let refused = run(&["booking", "save", path.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(3));
    let saved = run(&[
        "booking",
        "save",
        path.to_str().unwrap(),
        "--yes",
        "--output",
        "text",
    ]);
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );
    assert!(String::from_utf8_lossy(&saved.stdout).contains("Booking saved · verified"));
    assert_eq!(reads.load(Ordering::SeqCst), 3);
    writes.assert();
}
