//! An exact, account-bound change to one activity on one calendar day.
use crate::{
    Error,
    config::normalize_base,
    timesheet::{Timesheet, validate_date},
};
use serde::{Deserialize, Serialize};

pub(crate) fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput {
        message: message.into(),
    }
}
pub(crate) fn conflict(message: impl Into<String>) -> Error {
    Error::Conflict {
        message: message.into(),
    }
}

/// Parse minutes, HhMM, or H:MM without floating point or rounding.
pub fn parse_duration(value: &str) -> Result<u32, Error> {
    let value = value.trim();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let minutes = if let Some(m) = value.strip_suffix('m') {
        if !digits(m) {
            return Err(invalid("use a duration such as 7h30, 7:30 or 450m"));
        }
        m.parse::<u32>().ok()
    } else if let Some((h, m)) = value.split_once('h').or_else(|| value.split_once(':')) {
        if !digits(h) || !(m.is_empty() && value.ends_with('h') || m.len() == 2 && digits(m)) {
            return Err(invalid("use a duration such as 7h30, 7:30 or 450m"));
        }
        let h = h.parse::<u32>().ok();
        let m = if m.is_empty() {
            Some(0)
        } else {
            m.parse::<u32>().ok()
        };
        h.zip(m)
            .filter(|(_, m)| *m < 60)
            .and_then(|(h, m)| h.checked_mul(60)?.checked_add(m))
    } else {
        None
    };
    minutes
        .filter(|m| *m <= 1440)
        .ok_or_else(|| invalid("duration must be between 0m and 24h00; use 7h30, 7:30 or 450m"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BookingEdit {
    pub version: u32,
    pub base_url: String,
    pub employee_id: String,
    pub activity_id: String,
    pub activity_name: String,
    pub date: String,
    pub before_minutes: u32,
    pub minutes: u32,
}
impl BookingEdit {
    pub fn from_sheet(
        sheet: &Timesheet,
        activity: &str,
        date: &str,
        minutes: u32,
    ) -> Result<Self, Error> {
        validate_date(date)?;
        if !date.starts_with(&format!("{}-", sheet.month)) {
            return Err(invalid("date does not belong to the loaded month"));
        }
        let project = sheet
            .projects
            .iter()
            .find(|p| p.id == activity)
            .ok_or_else(|| invalid("activity is not present in this timesheet"))?;
        let before = *project
            .days
            .get(date)
            .ok_or_else(|| invalid("date is missing from the activity"))?;
        if before != minutes && project.locked_days.contains(date) {
            return Err(conflict("this booking is locked in Yoobi"));
        }
        let edit = Self {
            version: 1,
            base_url: sheet.base_url.clone(),
            employee_id: sheet.employee_id.clone(),
            activity_id: activity.into(),
            activity_name: project.name.clone(),
            date: date.into(),
            before_minutes: before,
            minutes,
        };
        edit.validate()?;
        Ok(edit)
    }
    pub fn validate(&self) -> Result<(), Error> {
        validate_date(&self.date)?;
        if self.version != 1
            || self.employee_id.trim().is_empty()
            || self.activity_id.trim().is_empty()
            || self.before_minutes > 1440
            || self.minutes > 1440
        {
            return Err(invalid(
                "invalid booking preview: requires version 1, account, activity and durations from 0 to 1440 minutes",
            ));
        }
        if normalize_base(&self.base_url)? != self.base_url {
            return Err(invalid("booking origin must be canonical"));
        }
        Ok(())
    }
    pub fn month(&self) -> &str {
        &self.date[..7]
    }
}
#[derive(Debug, Serialize)]
pub struct BookingResult {
    pub verified: bool,
    pub changed: bool,
    pub booking: BookingEdit,
    pub timesheet: Timesheet,
}
