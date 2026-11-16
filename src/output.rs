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
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}
pub fn text(value: &Value) -> String {
    if let Some(sheet) = value.get("timesheet") {
        return format!(
            "{} · verified\n\n{}",
            if value["changed"] == true {
                "Booking saved"
            } else {
                "Booking already matches"
            },
            text(sheet)
        );
    }
    if value.get("before_minutes").is_some() {
        return format!(
            "Booking preview · no changes saved\n\n{}\n{}\n{} → {}\n\nSave JSON output to a file, then use booking save FILE --yes.",
            clean(value["activity_name"].as_str().unwrap_or("Activity")),
            value["date"].as_str().unwrap_or(""),
            duration(value["before_minutes"].as_i64().unwrap_or(0)),
            duration(value["minutes"].as_i64().unwrap_or(0))
        );
    }
    if value["saved"] == true && value["verified"] == true {
        return if value["cleanup_pending"] == true {
            "Signed in · password saved in OS credential store. Previous credential cleanup failed."
        } else {
            "Signed in · password saved in OS credential store"
        }
        .into();
    }
    if value.get("removed").is_some() {
        return if value["removed"] == true {
            "Saved login removed"
        } else {
            "No saved login"
        }
        .into();
    }
    if let Some(configured) = value["configured"].as_bool() {
        return if configured {
            "Credentials configured · run timesheet to verify login"
        } else {
            "No saved login · run yoobi-cli auth login"
        }
        .into();
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
    serde_json::to_string_pretty(value).expect("JSON serializes")
}
