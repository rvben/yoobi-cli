//! Deterministic booking plans. Durations are integer minutes throughout.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Timesheet {
    pub base_url: String,
    pub employee_id: String,
    pub month: String,
    pub total_minutes: u32,
    pub projects: Vec<Project>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub days: BTreeMap<String, u32>,
    #[serde(default)]
    pub locked_days: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkInput {
    pub month: String,
    pub project_id: String,
    #[serde(default)]
    pub opening_balance_minutes: i64,
    pub days: Vec<WorkDay>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkDay {
    pub date: String,
    pub actual_minutes: u32,
    /// Explicitly choose 4 or 8 hours instead of nearest half/full day.
    pub hours: Option<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub base_url: String,
    pub employee_id: String,
    pub month: String,
    pub project_id: String,
    #[serde(default)]
    pub opening_balance_minutes: i64,
    pub days: Vec<PlannedDay>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlannedDay {
    pub date: String,
    pub actual_minutes: u32,
    pub before_minutes: u32,
    pub hours: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyResult {
    pub verified: bool,
    pub changed_days: usize,
    pub timesheet: Timesheet,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    #[serde(default)]
    pub opening_balance_minutes: i64,
    pub days: Vec<LedgerDay>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerDay {
    pub date: String,
    pub actual_minutes: u32,
    pub booked_minutes: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Balance {
    pub actual_minutes: i64,
    pub booked_minutes: i64,
    pub opening_balance_minutes: i64,
    /// Positive means worked hours not yet booked; negative means hours owed.
    pub balance_minutes: i64,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidInput {
        message: message.into(),
    }
}

pub fn validate_month(month: &str) -> Result<(), Error> {
    let bytes = month.as_bytes();
    if bytes.len() != 7
        || bytes[4] != b'-'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid("month must be YYYY-MM"));
    }
    let year: u32 = month[..4].parse().map_err(|_| invalid("invalid year"))?;
    let month: u32 = month[5..].parse().map_err(|_| invalid("invalid month"))?;
    if year == 0 || !(1..=12).contains(&month) {
        return Err(invalid("month must be a valid YYYY-MM"));
    }
    Ok(())
}
pub fn validate_date(date: &str) -> Result<(), Error> {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || !date.is_ascii()
        || bytes[7] != b'-'
        || !bytes[8..].iter().all(u8::is_ascii_digit)
    {
        return Err(invalid("date must be YYYY-MM-DD"));
    }
    validate_month(&date[..7])?;
    let day: u32 = date[8..].parse().map_err(|_| invalid("invalid day"))?;
    let max = days_in_month(&date[..7])?;
    if day == 0 || day > max {
        return Err(invalid("date is not a calendar day"));
    }
    Ok(())
}
/// Number of calendar days in a validated YYYY-MM period.
pub fn days_in_month(month: &str) -> Result<u32, Error> {
    validate_month(month)?;
    let year: u32 = month[..4].parse().map_err(|_| invalid("invalid year"))?;
    let month: u32 = month[5..].parse().map_err(|_| invalid("invalid month"))?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    Ok(match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    })
}
fn validate_duration(minutes: u32) -> Result<(), Error> {
    if minutes > 24 * 60 {
        return Err(invalid("a day cannot exceed 1440 minutes"));
    }
    Ok(())
}
fn validate_hours(hours: u8) -> Result<(), Error> {
    if ![4, 8].contains(&hours) {
        return Err(invalid("booking hours must be 4 or 8"));
    }
    Ok(())
}

impl Plan {
    pub fn build(input: WorkInput, sheet: &Timesheet) -> Result<Self, Error> {
        validate_month(&input.month)?;
        if input.month != sheet.month {
            return Err(invalid("input month differs from timesheet month"));
        }
        let project = sheet
            .projects
            .iter()
            .find(|p| p.id == input.project_id)
            .ok_or_else(|| invalid("project_id is not present in this timesheet"))?;
        let mut days = Vec::new();
        for day in input.days {
            validate_date(&day.date)?;
            validate_duration(day.actual_minutes)?;
            if !day.date.starts_with(&format!("{}-", input.month)) {
                return Err(invalid("every day must belong to the input month"));
            }
            if day.actual_minutes == 0 {
                return Err(invalid(
                    "zero-hour days must be omitted from a booking plan",
                ));
            }
            let hours = day
                .hours
                .unwrap_or(if day.actual_minutes >= 360 { 8 } else { 4 });
            validate_hours(hours)?;
            let before_minutes = *project.days.get(&day.date).unwrap_or(&0);
            if before_minutes != u32::from(hours) * 60 && project.locked_days.contains(&day.date) {
                return Err(Error::Conflict {
                    message: format!("{} is locked", day.date),
                });
            }
            days.push(PlannedDay {
                date: day.date,
                actual_minutes: day.actual_minutes,
                before_minutes,
                hours,
            });
        }
        days.sort_by(|a, b| a.date.cmp(&b.date));
        let plan = Self {
            version: 1,
            base_url: sheet.base_url.clone(),
            employee_id: sheet.employee_id.clone(),
            month: input.month,
            project_id: input.project_id,
            opening_balance_minutes: input.opening_balance_minutes,
            days,
        };
        plan.validate()?;
        Ok(plan)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != 1 {
            return Err(invalid("unsupported plan version"));
        }
        validate_month(&self.month)?;
        if self.base_url.is_empty() || self.employee_id.is_empty() || self.project_id.is_empty() {
            return Err(invalid(
                "plan requires base_url, employee_id and project_id",
            ));
        }
        if self.days.is_empty() {
            return Err(invalid("booking plan is empty"));
        }
        let mut seen = BTreeSet::new();
        for day in &self.days {
            validate_date(&day.date)?;
            if !day.date.starts_with(&format!("{}-", self.month)) {
                return Err(invalid("plan contains a day outside its month"));
            }
            if !seen.insert(&day.date) {
                return Err(invalid("plan contains duplicate dates"));
            }
            validate_duration(day.actual_minutes)?;
            validate_duration(day.before_minutes)?;
            validate_hours(day.hours)?;
            if day.actual_minutes == 0 {
                return Err(invalid(
                    "zero-hour days must be omitted from a booking plan",
                ));
            }
        }
        Ok(())
    }
    pub fn balance(&self) -> Result<Balance, Error> {
        self.validate()?;
        Ledger {
            opening_balance_minutes: self.opening_balance_minutes,
            days: self
                .days
                .iter()
                .map(|d| LedgerDay {
                    date: d.date.clone(),
                    actual_minutes: d.actual_minutes,
                    booked_minutes: u32::from(d.hours) * 60,
                })
                .collect(),
        }
        .balance()
    }
}
impl Ledger {
    pub fn balance(&self) -> Result<Balance, Error> {
        let mut seen = BTreeSet::new();
        let mut actual = 0i64;
        let mut booked = 0i64;
        for day in &self.days {
            validate_date(&day.date)?;
            if !seen.insert(&day.date) {
                return Err(invalid("ledger contains duplicate dates"));
            }
            validate_duration(day.actual_minutes)?;
            validate_duration(day.booked_minutes)?;
            actual = actual
                .checked_add(i64::from(day.actual_minutes))
                .ok_or_else(|| invalid("actual duration overflow"))?;
            booked = booked
                .checked_add(i64::from(day.booked_minutes))
                .ok_or_else(|| invalid("booked duration overflow"))?;
        }
        let balance_minutes = self
            .opening_balance_minutes
            .checked_add(actual)
            .and_then(|v| v.checked_sub(booked))
            .ok_or_else(|| invalid("balance overflow"))?;
        Ok(Balance {
            actual_minutes: actual,
            booked_minutes: booked,
            opening_balance_minutes: self.opening_balance_minutes,
            balance_minutes,
        })
    }
}
