//! Time conversions. ZIP stores DOS local time (no zone), so we convert through the Windows
//! time-zone APIs to match what Explorer shows.

#[cfg(windows)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
#[cfg(windows)]
use windows::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime,
};

#[cfg(windows)]
/// 100 ns ticks between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_TICKS: u64 = 116_444_736_000_000_000;

#[cfg(windows)]
pub fn systemtime_to_filetime(t: SystemTime) -> Option<FILETIME> {
    let d = t.duration_since(UNIX_EPOCH).ok()?;
    let ticks = EPOCH_DIFF_TICKS + (d.as_nanos() / 100) as u64;
    Some(FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 })
}

#[cfg(windows)]
pub fn filetime_to_systemtime(ft: FILETIME) -> Option<SystemTime> {
    let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    let since = ticks.checked_sub(EPOCH_DIFF_TICKS)?;
    Some(UNIX_EPOCH + Duration::from_nanos(since.checked_mul(100)?))
}

pub fn unix_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// DOS local time fields -> `SystemTime` (UTC based).
#[cfg(windows)]
pub fn dos_local_to_systemtime(y: u16, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Option<SystemTime> {
    let local = SYSTEMTIME {
        wYear: y,
        wMonth: mo as u16,
        wDayOfWeek: 0,
        wDay: d as u16,
        wHour: h as u16,
        wMinute: mi as u16,
        wSecond: s as u16,
        wMilliseconds: 0,
    };
    let mut utc = SYSTEMTIME::default();
    unsafe {
        TzSpecificLocalTimeToSystemTime(None, &local, &mut utc).ok()?;
        let mut ft = FILETIME::default();
        SystemTimeToFileTime(&utc, &mut ft).ok()?;
        filetime_to_systemtime(ft)
    }
}

/// `SystemTime` -> local (year, month, day, hour, minute, second).
#[cfg(windows)]
pub fn systemtime_to_local_fields(t: SystemTime) -> Option<(u16, u8, u8, u8, u8, u8)> {
    let ft = systemtime_to_filetime(t)?;
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some((local.wYear, local.wMonth as u8, local.wDay as u8, local.wHour as u8, local.wMinute as u8, local.wSecond as u8))
}

/// DOS local time fields -> `SystemTime` (UTC based), through the system time zone.
#[cfg(unix)]
pub fn dos_local_to_systemtime(y: u16, mo: u8, d: u8, h: u8, mi: u8, s: u8) -> Option<SystemTime> {
    use chrono::{Local, NaiveDate, TimeZone};
    let naive = NaiveDate::from_ymd_opt(y.into(), mo.into(), d.into())?.and_hms_opt(h.into(), mi.into(), s.into())?;
    // a time inside a DST gap doesn't exist locally: take the earliest valid reading
    let local = Local.from_local_datetime(&naive).earliest()?;
    Some(SystemTime::from(local))
}

/// `SystemTime` -> local (year, month, day, hour, minute, second).
#[cfg(unix)]
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
