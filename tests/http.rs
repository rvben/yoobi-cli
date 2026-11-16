use mockito::{Matcher, Server};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use yoobi_cli::{
    api::Client,
    config::Config,
    plan::{Plan, PlannedDay},
};

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
fn plan(server: &Server) -> Plan {
    Plan {
        version: 1,
        base_url: server.url(),
        employee_id: "employee".into(),
        month: "2026-11".into(),
        project_id: "activity".into(),
        opening_balance_minutes: 0,
        days: vec![PlannedDay {
            date: "2026-11-02".into(),
            actual_minutes: 345,
            before_minutes: 0,
            hours: 8,
        }],
    }
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
fn saves_exact_payload_and_verifies_fresh_read() {
    let mut s = Server::new();
    let c = login(&mut s);
    let p = plan(&s);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    s.mock("POST", "/timesheet/timesheet")
        .with_body_from_request(move |_| {
            grid(
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    0
                } else {
                    8
                },
                false,
            )
            .into_bytes()
        })
        .expect(2)
        .create();
    let payload = json!([{"key":"activity-2026-11-02","value":[{"time":8,"timeMsg":"","p":1,"a":1,"d":2,"ref":"","timediff":8.0,"pID":"project","bFromPlanning":false,"timedifforiginal":8.0}]}]);
    let save = s
        .mock("POST", "/declaration/saveDeclarations")
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("json".into(), payload.to_string()),
            Matcher::UrlEncoded("employeeID".into(), "employee".into()),
            Matcher::UrlEncoded("directSave".into(), "true".into()),
        ]))
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .expect(1)
        .create();
    let result = c.apply(&p).unwrap();
    assert!(result.verified);
    assert_eq!(result.changed_days, 1);
    save.assert();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[test]
fn stale_and_locked_plans_never_save() {
    for (hours, locked) in [(4, false), (0, true)] {
        let mut s = Server::new();
        let c = login(&mut s);
        let p = plan(&s);
        s.mock("POST", "/timesheet/timesheet")
            .with_body(grid(hours, locked))
            .create();
        let save = s
            .mock("POST", "/declaration/saveDeclarations")
            .expect(0)
            .create();
        assert!(matches!(
            c.apply(&p),
            Err(yoobi_cli::Error::Conflict { .. })
        ));
        save.assert();
    }
}
#[test]
fn matching_locked_days_are_idempotent() {
    let mut s = Server::new();
    let c = login(&mut s);
    let p = plan(&s);
    s.mock("POST", "/timesheet/timesheet")
        .with_body(grid(8, true))
        .expect(2)
        .create();
    let save = s
        .mock("POST", "/declaration/saveDeclarations")
        .expect(0)
        .create();
    assert_eq!(c.apply(&p).unwrap().changed_days, 0);
    save.assert();
}
#[test]
fn successful_acknowledgement_is_not_enough() {
    let mut s = Server::new();
    let c = login(&mut s);
    let p = plan(&s);
    s.mock("POST", "/timesheet/timesheet")
        .with_body(grid(0, false))
        .expect(2)
        .create();
    s.mock("POST", "/declaration/saveDeclarations")
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .create();
    assert!(matches!(
        c.apply(&p),
        Err(yoobi_cli::Error::Conflict { .. })
    ));
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
fn ambiguous_and_rejected_saves_are_not_retried() {
    for body in [
        r#"{"messages":[{"type":"error"}]}"#,
        r#"{"messages":[]}"#,
        "not json",
    ] {
        let mut s = Server::new();
        let c = login(&mut s);
        let p = plan(&s);
        s.mock("POST", "/timesheet/timesheet")
            .with_body(grid(0, false))
            .expect(1)
            .create();
        let save = s
            .mock("POST", "/declaration/saveDeclarations")
            .with_body(body)
            .expect(1)
            .create();
        assert!(c.apply(&p).is_err());
        save.assert();
    }
}
#[test]
fn save_server_failure_is_uncertain_and_not_retried() {
    let mut s = Server::new();
    let c = login(&mut s);
    let p = plan(&s);
    s.mock("POST", "/timesheet/timesheet")
        .with_body(grid(0, false))
        .create();
    let save = s
        .mock("POST", "/declaration/saveDeclarations")
        .with_status(503)
        .with_body("private failure message")
        .expect(1)
        .create();
    let e = c.apply(&p).unwrap_err();
    assert!(e.to_string().contains("may already have changed"));
    assert!(!e.to_string().contains("private failure"));
    save.assert();
}
#[test]
fn foreign_employee_or_origin_refused_before_timesheet_request() {
    let mut s = Server::new();
    let c = login(&mut s);
    let mut p = plan(&s);
    let read = s.mock("POST", "/timesheet/timesheet").expect(0).create();
    p.employee_id = "another-employee".into();
    assert!(c.apply(&p).is_err());
    p.employee_id = "employee".into();
    p.base_url = "https://other.invalid".into();
    assert!(c.apply(&p).is_err());
    read.assert();
}
#[test]
fn notes_and_references_are_preserved_and_unrelated_changes_detected() {
    let mut s = Server::new();
    let c = login(&mut s);
    let p = plan(&s);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    s.mock("POST", "/timesheet/timesheet")
        .with_body_from_request(move |_| {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                grid(0, false)
                    .replace("\"timeMsg\":\"\"", "\"timeMsg\":\"existing note\"")
                    .replace("\"ref\":\"\"", "\"ref\":\"existing ref\"")
            } else {
                grid(8, false)
                    .replace("\"monthTotal\":8", "\"monthTotal\":12")
                    .replace("daylytotal'>8", "daylytotal'>12")
                    .replace(
                        "\"dtdate\":\"2026-11-03\",\"ref\":\"\",\"time\":0",
                        "\"dtdate\":\"2026-11-03\",\"ref\":\"\",\"time\":4",
                    )
            }
            .into_bytes()
        })
        .create();
    let save = s
        .mock("POST", "/declaration/saveDeclarations")
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex("existing\\+note".into()),
            Matcher::Regex("existing\\+ref".into()),
        ]))
        .with_body(r#"{"messages":[{"type":"success"}]}"#)
        .expect(1)
        .create();
    assert!(matches!(
        c.apply(&p),
        Err(yoobi_cli::Error::Conflict { .. })
    ));
    save.assert();
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
