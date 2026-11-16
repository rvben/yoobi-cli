//! Human-readable tables, kept separate from the machine contract.
use serde_json::Value;

pub fn duration(minutes: i64) -> String {
    let m = minutes.unsigned_abs();
    format!(
        "{}{}h{:02}",
        if minutes < 0 { "−" } else { "" },
        m / 60,
        m % 60
    )
}
pub fn magnitude(minutes: i64) -> String {
    let m = minutes.unsigned_abs();
    format!("{}h{:02}", m / 60, m % 60)
}
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}
pub fn text(value: &Value) -> String {
    if let Some(minutes) = value["balance_minutes"].as_i64() {
        return format!(
            "{} {}\n\nActual          {:>10}\nBooked          {:>10}\nOpening balance {:>10}",
            magnitude(minutes),
            if minutes < 0 { "debt" } else { "surplus" },
            duration(value["actual_minutes"].as_i64().unwrap_or(0)),
            duration(value["booked_minutes"].as_i64().unwrap_or(0)),
            duration(value["opening_balance_minutes"].as_i64().unwrap_or(0))
        );
    }
    if let Some(configured) = value["configured"].as_bool() {
        return if configured {
            "Credentials configured · run timesheet to verify login"
        } else {
            "Credentials missing · set YOOBI_BASE_URL, YOOBI_USERNAME and a password source"
        }
        .into();
    }
    if let Some(sheet) = value.get("timesheet") {
        return format!(
            "Verified · {} day(s) changed\n\n{}",
            value["changed_days"],
            text(sheet)
        );
    }
    if value["dry_run"] == true {
        return format!(
            "Preview only · no requests sent\n\n{}\n\n{}",
            text(&value["plan"]),
            text(&value["balance"])
        );
    }
    if let Some(projects) = value["projects"].as_array() {
        let mut out = format!("YOOBI  {}", value["month"].as_str().unwrap_or(""));
        if let Some(total) = value["total_minutes"].as_i64() {
            out.push_str(&format!(" · {} booked", duration(total)));
        }
        for project in projects {
            out.push_str(&format!(
                "\n\n{}\n  {}",
                clean(project["name"].as_str().unwrap_or("Activity")),
                project["id"].as_str().unwrap_or("")
            ));
            if let Some(days) = project["days"].as_object() {
                out.push_str("\n  DATE          BOOKED    STATE");
                for (date, minutes) in days {
                    let m = minutes.as_i64().unwrap_or(0);
                    if m == 0 {
                        continue;
                    }
                    let locked = project["locked_days"]
                        .as_array()
                        .is_some_and(|d| d.iter().any(|d| d == date));
                    out.push_str(&format!(
                        "\n  {date}    {:>6}    {}",
                        duration(m),
                        if locked { "locked" } else { "open" }
                    ));
                }
            }
        }
        if projects.is_empty() {
            out.push_str("\n\nNo activities in this month.");
        }
        return out;
    }
    if let Some(days) = value["days"].as_array() {
        let mut out = format!(
            "Booking plan · {}\n\nDATE          ACTUAL    CURRENT    PLANNED",
            value["month"].as_str().unwrap_or("")
        );
        for day in days {
            out.push_str(&format!(
                "\n{}    {:>6}     {:>6}     {:>6}",
                day["date"].as_str().unwrap_or(""),
                duration(day["actual_minutes"].as_i64().unwrap_or(0)),
                duration(day["before_minutes"].as_i64().unwrap_or(0)),
                duration(day["hours"].as_i64().unwrap_or(0) * 60)
            ));
        }
        return out;
    }
    serde_json::to_string_pretty(value).expect("JSON serializes")
}
