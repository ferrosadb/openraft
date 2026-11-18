use std::fmt;
use std::sync::Once;

use jiff::civil::DateTime;
use jiff::tz::Offset;
use jiff::tz::TimeZone;
use jiff::Timestamp;

use crate::Instant;

/// Display `Instant` in human readable format.
pub(crate) struct DisplayInstant<'a, T, const SIMPLE: bool = true, const LOCAL: bool = true>(pub &'a T);

impl<'a, T, const SIMPLE: bool, const LOCAL: bool> DisplayInstant<'a, T, SIMPLE, LOCAL> {
    /// Display `Instant` in full format: with date and timezone: the format is
    /// "%Y-%m-%dT%H:%M:%S%.6fZ%z"
    #[allow(dead_code)]
    pub fn full(self) -> DisplayInstant<'a, T, false, LOCAL> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in simple format: without date and timezone: the format is "%H:%M:%S%.6f"
    #[allow(dead_code)]
    pub fn simple(self) -> DisplayInstant<'a, T, true, LOCAL> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in local timezone.
    #[allow(dead_code)]
    pub fn local(self) -> DisplayInstant<'a, T, SIMPLE, true> {
        DisplayInstant(self.0)
    }

    /// Display `Instant` in UTC timezone.
    #[allow(dead_code)]
    pub fn utc(self) -> DisplayInstant<'a, T, SIMPLE, false> {
        DisplayInstant(self.0)
    }
}

impl<T, const SIMPLE: bool, const LOCAL: bool> fmt::Display for DisplayInstant<'_, T, SIMPLE, LOCAL>
where T: Instant
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Convert Instant to SystemTime
        let sys_t = {
            let sys_now = std::time::SystemTime::now();
            let now = T::now();

            if &now >= self.0 {
                let d = now - *self.0;
                sys_now - d
            } else {
                let d = *self.0 - now;
                sys_now + d
            }
        };

        let timestamp = match Timestamp::try_from(sys_t) {
            Ok(timestamp) => timestamp,
            Err(err) => return write!(f, "<instant out of range: {}>", err),
        };

        let offset = if LOCAL {
            local_time_zone().to_offset(timestamp)
        } else {
            Offset::UTC
        };

        write_datetime(f, offset.to_datetime(timestamp), offset, SIMPLE)
    }
}

/// Returns the system time zone, resolved the way `chrono::Local` resolved it.
///
/// The `TZ` environment variable wins; if it is unset, the system default
/// (`/etc/localtime` on Unix) is used. Like `chrono::Local`:
/// - if `TZ` is set but invalid, `/etc/localtime` is used (Unix only);
/// - if no time zone can be determined, UTC is used.
///
/// Unlike `chrono::Local`, which applied those fallbacks silently, the first
/// fallback is reported with a `tracing` warning. The time zone is resolved
/// again on every call, so fixing the configuration takes effect without a
/// restart (jiff caches a successfully resolved system time zone for up to
/// 5 minutes).
fn local_time_zone() -> TimeZone {
    match TimeZone::try_system() {
        Ok(tz) => tz,
        Err(err) => fallback_time_zone(err),
    }
}

/// Warns, at most once per process, that the system time zone could not be
/// determined and which fallback is used instead.
fn warn_fallback_once(message: impl FnOnce() -> String) {
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| tracing::warn!("{}", message()));
}

#[cfg(unix)]
fn fallback_time_zone(err: jiff::Error) -> TimeZone {
    const LOCALTIME: &str = "/etc/localtime";

    let fallback = std::fs::read(LOCALTIME)
        .map_err(|e| e.to_string())
        .and_then(|data| TimeZone::tzif("Local", &data).map_err(|e| e.to_string()));

    match fallback {
        Ok(tz) => {
            warn_fallback_once(|| {
                format!(
                    "failed to determine the system time zone ({}), displaying instants in the time zone of {}",
                    err, LOCALTIME
                )
            });
            tz
        }
        Err(fallback_err) => {
            warn_fallback_once(|| {
                format!(
                    "failed to determine the system time zone ({}) and to read {} ({}), displaying instants in UTC",
                    err, LOCALTIME, fallback_err
                )
            });
            TimeZone::UTC
        }
    }
}

#[cfg(not(unix))]
fn fallback_time_zone(err: jiff::Error) -> TimeZone {
    warn_fallback_once(|| {
        format!(
            "failed to determine the system time zone ({}), displaying instants in UTC",
            err
        )
    });
    TimeZone::UTC
}

/// Writes `datetime` as chrono 0.4 formatted it with `"%H:%M:%S%.6f"`
/// (`simple`) or `"%Y-%m-%dT%H:%M:%S%.6fZ%z"` (full).
fn write_datetime(f: &mut fmt::Formatter<'_>, datetime: DateTime, offset: Offset, simple: bool) -> fmt::Result {
    if !simple {
        let year = datetime.year();
        if (0..=9999).contains(&year) {
            write!(f, "{:04}", year)?;
        } else {
            // chrono requires an explicit sign for years outside 0..=9999.
            write!(f, "{:+05}", year)?;
        }
        write!(f, "-{:02}-{:02}T", datetime.month(), datetime.day())?;
    }

    // `%.6f`: microseconds, truncated.
    write!(
        f,
        "{:02}:{:02}:{:02}.{:06}",
        datetime.hour(),
        datetime.minute(),
        datetime.second(),
        datetime.subsec_nanosecond() / 1_000
    )?;

    if !simple {
        f.write_str("Z")?;
        write_offset(f, offset)?;
    }
    Ok(())
}

/// Writes `offset` as chrono 0.4's `%z` did: `+hhmm`, with the seconds of
/// the offset rounded to the nearest minute.
fn write_offset(f: &mut fmt::Formatter<'_>, offset: Offset) -> fmt::Result {
    let seconds = offset.seconds();
    let (sign, abs_seconds) = if seconds < 0 { ('-', -seconds) } else { ('+', seconds) };
    let minutes = (abs_seconds + 30) / 60;
    write!(f, "{}{:02}{:02}", sign, minutes / 60, minutes % 60)
}

pub(crate) trait DisplayInstantExt<'a, T> {
    fn display(&'a self) -> DisplayInstant<'a, T, true>;
}

impl<T> DisplayInstantExt<'_, T> for T
where T: Instant
{
    fn display(&self) -> DisplayInstant<'_, T, true> {
        DisplayInstant(self)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use jiff::tz::Offset;
    use jiff::Timestamp;

    use super::write_datetime;
    use crate::display_ext::DisplayInstantExt;
    use crate::TokioInstant;

    /// Check the result by a human.
    #[test]
    fn test_display_instant() -> anyhow::Result<()> {
        let now = TokioInstant::now();
        println!("now: {}", now.display());
        println!("now: {}", now.display().full());
        println!("now: {}", now.display().full().simple());
        println!("now: {}", now.display().utc());
        println!("now: {}", now.display().utc().local());
        println!("now: {}", now.display().utc().full());
        Ok(())
    }

    struct Fmt {
        timestamp: Timestamp,
        offset: Offset,
        simple: bool,
    }

    impl fmt::Display for Fmt {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write_datetime(f, self.offset.to_datetime(self.timestamp), self.offset, self.simple)
        }
    }

    fn render(second: i64, nanosecond: i32, offset_seconds: i32, simple: bool) -> String {
        Fmt {
            timestamp: Timestamp::new(second, nanosecond).unwrap(),
            offset: Offset::from_seconds(offset_seconds).unwrap(),
            simple,
        }
        .to_string()
    }

    /// The output must match chrono 0.4's `"%H:%M:%S%.6f"` and
    /// `"%Y-%m-%dT%H:%M:%S%.6fZ%z"`, which this module used before.
    #[test]
    fn test_write_datetime_matches_chrono_format() {
        // 2022-02-22T20:22:22.123456789Z
        let t = 1_645_561_342;
        let ns = 123_456_789;

        assert_eq!(render(t, ns, 0, true), "20:22:22.123456");
        assert_eq!(render(t, ns, 0, false), "2022-02-22T20:22:22.123456Z+0000");
        assert_eq!(render(t, 0, 0, false), "2022-02-22T20:22:22.000000Z+0000");
        assert_eq!(render(t, 999, 0, true), "20:22:22.000000");

        // Positive and negative offsets, and crossing a date boundary.
        assert_eq!(render(t, ns, 8 * 3600, true), "04:22:22.123456");
        assert_eq!(render(t, ns, 8 * 3600, false), "2022-02-23T04:22:22.123456Z+0800");
        assert_eq!(
            render(t, ns, -(5 * 3600 + 30 * 60), false),
            "2022-02-22T14:52:22.123456Z-0530"
        );
        assert_eq!(
            render(t, ns, 13 * 3600 + 45 * 60, false),
            "2022-02-23T10:07:22.123456Z+1345"
        );

        // Offset seconds are rounded to the nearest minute, as chrono's `%z` did.
        assert_eq!(render(t, 0, 3600 + 29, false), "2022-02-22T21:22:51.000000Z+0100");
        assert_eq!(render(t, 0, 3600 + 30, false), "2022-02-22T21:22:52.000000Z+0101");
        assert_eq!(render(t, 0, -(3600 + 30), false), "2022-02-22T19:21:52.000000Z-0101");

        // Years below 1000 are zero padded, negative years carry a sign.
        // 0001-02-03T04:05:06Z
        assert_eq!(render(-62_132_730_894, 0, 0, false), "0001-02-03T04:05:06.000000Z+0000");
        // -0001-12-31T23:59:59Z
        assert_eq!(
            render(-62_167_219_201, 0, 0, false),
            "-0001-12-31T23:59:59.000000Z+0000"
        );
    }

    #[test]
    fn test_display_instant_utc_shape() {
        let now = TokioInstant::now();

        let simple = now.display().utc().to_string();
        assert_eq!(simple.len(), "20:22:22.123456".len(), "{}", simple);

        let full = now.display().utc().full().to_string();
        assert_eq!(full.len(), "2022-02-22T20:22:22.123456Z+0000".len(), "{}", full);
        assert!(full.ends_with("Z+0000"), "{}", full);

        let local = now.display().full().to_string();
        assert_eq!(local.len(), full.len(), "{}", local);
    }
}
