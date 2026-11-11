//! Time conversions. ZIP stores DOS local time (no zone), so we convert through the system
//! time zone (as Explorer does on Windows, where most zips come from).

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// DOS local time fields -> `SystemTime` (UTC based), through the system time zone.
pub fn dos_local_to_systemtime(y: u16, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Option<SystemTime> {
    use chrono::{Local, NaiveDate, TimeZone};
    let naive = NaiveDate::from_ymd_opt(y.into(), mo.into(), d.into())?.and_hms_opt(h.into(), mi.into(), s.into())?;
    // a time inside a DST gap doesn't exist locally: take the earliest valid reading
    let local = Local.from_local_datetime(&naive).earliest()?;
    Some(SystemTime::from(local))
}

/// `SystemTime` -> local (year, month, day, hour, minute, second).
pub fn systemtime_to_local_fields(t: SystemTime) -> Option<(u16, u8, u8, u8, u8, u8)> {
    use chrono::{Datelike, Local, Timelike};
    let local: chrono::DateTime<Local> = t.into();
    Some((
        u16::try_from(local.year()).ok()?,
        local.month() as u8,
        local.day() as u8,
        local.hour() as u8,
        local.minute() as u8,
        local.second() as u8,
    ))
}
