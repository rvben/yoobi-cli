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
