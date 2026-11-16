//! Yoobi timesheet data and calendar validation.
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
