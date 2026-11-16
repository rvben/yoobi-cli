//! Native HTTP adapter for Yoobi's classic timesheet protocol (2.30.2).
use crate::{
    Error,
    config::Config,
    plan::{ApplyResult, Plan, Project, Timesheet},
};
use regex::Regex;
use reqwest::{
    blocking::{Client as Http, Response},
    redirect::Policy,
};
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    time::Duration,
};

pub struct Client {
    http: Http,
    config: Config,
    employee_id: String,
}
#[derive(Clone)]
struct Cell {
    project: String,
    p: usize,
    a: usize,
    d: usize,
    value: Value,
    writable: bool,
}
struct Snapshot {
    sheet: Timesheet,
    cells: BTreeMap<(String, String), Cell>,
}
fn protocol(message: impl Into<String>) -> Error {
    Error::Protocol {
        message: message.into(),
    }
}
fn conflict(message: impl Into<String>) -> Error {
    Error::Conflict {
        message: message.into(),
    }
}
fn selector(s: &str) -> Selector {
    Selector::parse(s).expect("constant CSS selector")
}
fn number(v: &Value) -> Result<u32, Error> {
    minutes(v, 1440.0)
}
fn minutes(v: &Value, maximum: f64) -> Result<u32, Error> {
    let n = v
        .as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .ok_or_else(|| protocol("invalid duration in Yoobi response"))?;
    let minutes = n * 60.0;
    if !minutes.is_finite()
        || !(0.0..=maximum).contains(&minutes)
        || (minutes - minutes.round()).abs() > 0.0001
    {
        return Err(protocol(
            "Yoobi duration is out of range or is not an exact minute",
        ));
    }
    Ok(minutes.round() as u32)
}
fn body(r: Response, step: &str) -> Result<String, Error> {
    if r.status() == reqwest::StatusCode::UNAUTHORIZED
        || r.status() == reqwest::StatusCode::FORBIDDEN
    {
        return Err(Error::Auth {
            message: format!("Yoobi refused {step}"),
        });
    }
    if !r.status().is_success() {
        return Err(Error::Network {
            message: format!("Yoobi {step} returned HTTP {}", r.status().as_u16()),
        });
    }
    let mut data = String::new();
    r.take(8 * 1024 * 1024 + 1)
        .read_to_string(&mut data)
        .map_err(|_| protocol(format!("cannot read Yoobi {step} response")))?;
    if data.len() > 8 * 1024 * 1024 {
        return Err(protocol("Yoobi response exceeds 8 MiB"));
    }
    Ok(data)
}
impl Client {
    pub fn new(config: &Config) -> Result<Self, Error> {
        let origin = url::Url::parse(&config.base_url)
            .map_err(|_| protocol("invalid configured origin"))?
            .origin();
        let http = Http::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("yoobi-cli/", env!("CARGO_PKG_VERSION")))
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 || attempt.url().origin() != origin {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| protocol("cannot initialize HTTP client"))?;
        let client = Self {
            http,
            config: config.clone(),
            employee_id: String::new(),
        };
        let login = client
            .http
            .post(format!("{}/security/doLogin", config.base_url))
            .form(&[
                ("username", config.username.as_str()),
                ("pass", config.password()),
                ("userLocale", "nl_NL"),
                ("_securedURL", ""),
            ])
            .send()
            .map_err(|_| Error::Network {
                message: "Yoobi login request failed; credentials were not retried".into(),
            })?;
        body(login, "login")?;
        let index = client.get("/timesheet/index")?;
        let doc = Html::parse_document(&index);
        let employee=doc.select(&selector("#employeeID")).next().and_then(|e|e.value().attr("value")).filter(|v|!v.is_empty()).ok_or_else(||Error::Auth {message:"Yoobi did not open a timesheet; check credentials or complete SSO in the supported client".into()})?.to_owned();
        Ok(Self {
            employee_id: employee,
            ..client
        })
    }
    fn get(&self, path: &str) -> Result<String, Error> {
        let r = self
            .http
            .get(format!("{}{path}", self.config.base_url))
            .send()
            .map_err(|_| Error::Network {
                message: "Yoobi read request failed".into(),
            })?;
        body(r, "read")
    }
    pub fn timesheet(&self, month: &str) -> Result<Timesheet, Error> {
        Ok(self.snapshot(month)?.sheet)
    }
    fn snapshot(&self, month: &str) -> Result<Snapshot, Error> {
        crate::plan::validate_month(month)?;
        let (year, m) = month
            .split_once('-')
            .ok_or_else(|| protocol("invalid month"))?;
        let m = m.trim_start_matches('0');
        let period = format!("{year}-{m}");
        // This POST is Yoobi's read-only HTML rendering route, not a save.
        let r = self
            .http
            .post(format!("{}/timesheet/timesheet", self.config.base_url))
            .form(&[
                ("month", m),
                ("year", year),
                ("period", period.as_str()),
                ("type", "timesheet"),
                ("employeeID", self.employee_id.as_str()),
                ("notInActivityID", ""),
                ("notInProjectID", ""),
            ])
            .send()
            .map_err(|_| Error::Network {
                message: "Yoobi timesheet read failed".into(),
            })?;
        let html = body(r, "timesheet read")?;
        parse_grid(&html, &self.config.base_url, &self.employee_id, month)
    }
    pub fn apply(&self, plan: &Plan) -> Result<ApplyResult, Error> {
        plan.validate()?;
        if plan.base_url != self.config.base_url || plan.employee_id != self.employee_id {
            return Err(conflict(
                "plan belongs to a different Yoobi origin or employee",
            ));
        }
        let before = self.snapshot(&plan.month)?;
        let project = before
            .sheet
            .projects
            .iter()
            .find(|p| p.id == plan.project_id)
            .ok_or_else(|| conflict("booking activity no longer exists"))?;
        let mut payload = Vec::new();
        for day in &plan.days {
            let cell = before
                .cells
                .get(&(plan.project_id.clone(), day.date.clone()))
                .ok_or_else(|| conflict(format!("{} is missing from the timesheet", day.date)))?;
            let minutes = *project
                .days
                .get(&day.date)
                .ok_or_else(|| conflict("missing day"))?;
            let desired = u32::from(day.hours) * 60;
            if minutes == desired {
                continue;
            }
            if minutes != day.before_minutes {
                return Err(conflict(format!(
                    "{} changed since the plan was created",
                    day.date
                )));
            }
            if !cell.writable {
                return Err(conflict(format!(
                    "{} is locked or requires unsupported booking details",
                    day.date
                )));
            }
            let diff = f64::from(desired) / 60.0 - f64::from(minutes) / 60.0;
            payload.push(json!({"key":format!("{}-{}",plan.project_id,day.date),"value":[{
                "time":day.hours,"timeMsg":cell.value.get("timeMsg").cloned().unwrap_or(json!("")),
                "p":cell.p,"a":cell.a,"d":cell.d,"ref":cell.value.get("ref").cloned().unwrap_or(json!("")),
                "timediff":diff,"pID":cell.project,"bFromPlanning":false,"timedifforiginal":diff
            }]}));
        }
        let count = payload.len();
        if count > 0 {
            let response=self.http.post(format!("{}/declaration/saveDeclarations",self.config.base_url))
                .form(&[("json",json!(payload).to_string()),("employeeID",self.employee_id.clone()),("directSave","true".into())])
                .send().map_err(|_|Error::Network {message:"Yoobi save was interrupted; some entries may have been saved. Read the timesheet before retrying".into()})?;
            let value: Value =
                serde_json::from_str(&body(response, "save (entries may already have changed)")?)
                    .map_err(|_| {
                    protocol("unrecognized save response; read the timesheet before retrying")
                })?;
            let messages = value["messages"]
                .as_array()
                .filter(|m| !m.is_empty())
                .ok_or_else(|| {
                    protocol(
                        "save response has no acknowledgement; read the timesheet before retrying",
                    )
                })?;
            if messages.iter().any(|m| m["type"] != "success") {
                return Err(conflict(
                    "Yoobi rejected all or part of the save; read the timesheet before retrying",
                ));
            }
        }
        let after = self.snapshot(&plan.month)?.sheet;
        let mut expected = before.sheet;
        let project = expected
            .projects
            .iter_mut()
            .find(|p| p.id == plan.project_id)
            .expect("validated project");
        for day in &plan.days {
            project
                .days
                .insert(day.date.clone(), u32::from(day.hours) * 60);
        }
        // Names and lock states can change independently. Check every booking, including untargeted days.
        if after.projects.len() != expected.projects.len()
            || after.projects.iter().any(|p| {
                expected
                    .projects
                    .iter()
                    .find(|e| e.id == p.id)
                    .is_none_or(|e| e.days != p.days)
            })
        {
            return Err(conflict(
                "saved timesheet does not match the plan or another booking changed; inspect it before retrying",
            ));
        }
        Ok(ApplyResult {
            verified: true,
            changed_days: count,
            timesheet: after,
        })
    }
}

fn parse_grid(html: &str, base: &str, employee: &str, month: &str) -> Result<Snapshot, Error> {
    let doc = Html::parse_document(html);
    let node = doc.select(&selector("#stData")).next().ok_or_else(|| {
        protocol("Yoobi did not return timesheet data; session expired or format changed")
    })?;
    let data: Value = serde_json::from_str(&node.text().collect::<String>())
        .map_err(|_| protocol("invalid structured timesheet data"))?;
    let raw_projects = data["projects"]
        .as_array()
        .ok_or_else(|| protocol("missing timesheet projects"))?;
    let date_re = Regex::new(r"date:\s*'(\d{4}-\d{2}-\d{2})'").expect("constant date regex");
    let mut projects = Vec::new();
    let mut cells = BTreeMap::new();
    let mut total = 0u32;
    for (p, raw) in raw_projects.iter().enumerate().skip(1) {
        let project_id = raw["pID"]
            .as_str()
            .ok_or_else(|| protocol("missing project ID"))?;
        let left_id = format!("project_{p}-l");
        let name = doc
            .select(&selector(".project"))
            .find(|n| n.value().id() == Some(left_id.as_str()))
            .and_then(|n| n.select(&selector(".fullName")).next())
            .map(|n| n.text().collect::<String>().trim().to_owned())
            .unwrap_or_else(|| project_id.to_owned());
        let acts = raw["activities"]
            .as_array()
            .ok_or_else(|| protocol("missing project activities"))?;
        for (a, act) in acts.iter().enumerate().skip(1) {
            let id = act["aID"]
                .as_str()
                .ok_or_else(|| protocol("missing activity ID"))?
                .to_owned();
            let target = format!("activity_{p}.{a}-d");
            let row = doc
                .select(&selector(".activity"))
                .find(|r| r.value().id() == Some(target.as_str()))
                .ok_or_else(|| protocol("activity grid missing"))?;
            let cls = row.value().attr("class").unwrap_or("");
            if !cls.contains("type:'time'") && !cls.contains("type: 'time'") {
                return Err(protocol(
                    "only hour-based timesheet activities are supported",
                ));
            }
            let restricted = act["allowOvertime"].as_i64().unwrap_or(0) != 0
                || !act["defaultOvertimeID"].as_str().unwrap_or("").is_empty()
                || cls.contains("bNoteRequired:true")
                || !cls.contains("alttype:[]");
            let mut locks = BTreeSet::new();
            let mut seen = BTreeSet::new();
            for cell in row.select(&selector("div.cell")) {
                let class = cell.value().attr("class").unwrap_or("");
                let date = date_re
                    .captures(class)
                    .map(|m| m[1].to_owned())
                    .ok_or_else(|| protocol("timesheet cell has no date"))?;
                if !seen.insert(date.clone()) {
                    return Err(protocol("duplicate timesheet day"));
                }
                if restricted
                    || class
                        .split_whitespace()
                        .any(|c| matches!(c, "disabled" | "closed" | "approved" | "wasApproved"))
                    || cell
                        .value()
                        .attr("data-warn")
                        .is_some_and(|v| !v.is_empty())
                {
                    locks.insert(date);
                }
            }
            let mut days = BTreeMap::new();
            for (d, entry) in act["decl"]
                .as_array()
                .ok_or_else(|| protocol("missing declarations"))?
                .iter()
                .enumerate()
                .skip(1)
            {
                let date = entry["dtdate"]
                    .as_str()
                    .ok_or_else(|| protocol("declaration has no date"))?;
                if !date.starts_with(&format!("{month}-")) {
                    continue;
                }
                crate::plan::validate_date(date)?;
                if !seen.contains(date) {
                    return Err(protocol("declaration missing from rendered grid"));
                }
                let minutes = number(&entry["time"])?;
                if days.insert(date.to_owned(), minutes).is_some() {
                    return Err(protocol("duplicate declaration"));
                }
                total = total
                    .checked_add(minutes)
                    .ok_or_else(|| protocol("timesheet total overflow"))?;
                if cells
                    .insert(
                        (id.clone(), date.to_owned()),
                        Cell {
                            project: project_id.to_owned(),
                            p,
                            a,
                            d,
                            value: entry.clone(),
                            writable: !locks.contains(date),
                        },
                    )
                    .is_some()
                {
                    return Err(protocol("ambiguous activity ID"));
                }
            }
            let month_days = crate::plan::days_in_month(month)?;
            if days.len() != month_days as usize {
                return Err(protocol(
                    "timesheet grid does not cover the requested month",
                ));
            }
            let title_id = format!("activity_{p}.{a}");
            let activity_name = doc
                .select(&selector("[title]"))
                .find(|n| n.value().id() == Some(title_id.as_str()))
                .and_then(|n| n.value().attr("title"))
                .unwrap_or(&id);
            projects.push(Project {
                id: id.clone(),
                name: format!("{name} / {activity_name}"),
                days,
                locked_days: locks.into_iter().filter(|d| d.starts_with(month)).collect(),
            });
        }
    }
    let official = minutes(&data["monthTotal"], f64::from(u32::MAX))?;
    let visible = doc
        .select(&selector(".layoutRightBottom .daylytotal"))
        .next()
        .ok_or_else(|| protocol("visible month total missing"))?
        .text()
        .collect::<String>()
        .trim()
        .replace(',', ".");
    let visible = minutes(&json!(visible), f64::from(u32::MAX))?;
    if total != official || visible != official {
        return Err(protocol("Yoobi month total disagrees with daily entries"));
    }
    Ok(Snapshot {
        sheet: Timesheet {
            base_url: base.into(),
            employee_id: employee.into(),
            month: month.into(),
            total_minutes: official,
            projects,
        },
        cells,
    })
}
