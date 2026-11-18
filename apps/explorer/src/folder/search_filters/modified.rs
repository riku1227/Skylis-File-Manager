use chrono::{DateTime, Days, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};
use explorer_core::name_search::{ModifiedFilter, ModifiedPeriod};
use std::time::SystemTime;

pub(super) fn filter_at<Tz: TimeZone>(
    period: ModifiedPeriod,
    now: DateTime<Tz>,
) -> Option<ModifiedFilter> {
    let today = now.date_naive();
    let preceding_days = match period {
        ModifiedPeriod::Today => 0,
        ModifiedPeriod::LastSevenDays => 6,
        ModifiedPeriod::LastThirtyDays => 29,
    };
    let zone = now.timezone();
    let resolve = |local| {
        zone.from_local_datetime(&local)
            .earliest()
            .map(SystemTime::from)
    };
    let start = day_start(today.checked_sub_days(Days::new(preceding_days))?, &resolve)?;
    let end = day_start(today.succ_opt()?, &resolve)?;
    Some(ModifiedFilter::new(period, start..end))
}

fn day_start(
    day: NaiveDate,
    resolve: &impl Fn(NaiveDateTime) -> Option<SystemTime>,
) -> Option<SystemTime> {
    let midnight = day.and_hms_opt(0, 0, 0)?;
    if let Some(start) = resolve(midnight) {
        return Some(start);
    }

    // A clock transition can skip midnight (or an entire civil date).
    // Find the first valid second after the gap, doing no extra work on an ordinary day.
    let at_second = |seconds| resolve(midnight.checked_add_signed(TimeDelta::seconds(seconds))?);
    let hour = (1..=48).find(|hour| at_second(hour * 3600).is_some())?;
    let (mut low, mut high) = ((hour - 1) * 3600, hour * 3600);
    while high - low > 1 {
        let mid = (low + high) / 2;
        if at_second(mid).is_some() {
            high = mid;
        } else {
            low = mid;
        }
    }
    at_second(high)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use explorer_core::{Entry, EntryKind, name_search::Filters};
    use std::path::PathBuf;

    #[test]
    fn presets_count_local_dates_and_exclude_the_start_of_tomorrow() {
        let zone = FixedOffset::east_opt(9 * 3600).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let now = zone
            .from_local_datetime(&today.and_hms_opt(18, 0, 0).unwrap())
            .unwrap();
        let matches_on = |filters: &Filters, date: NaiveDate, hour| {
            let at = zone
                .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap())
                .unwrap();
            filters.matches(&Entry::virtual_entry(
                "example".into(),
                PathBuf::new(),
                EntryKind::File,
                None,
                Some(at.into()),
            ))
        };
        for (period, preceding) in [
            (ModifiedPeriod::Today, 0),
            (ModifiedPeriod::LastSevenDays, 6),
            (ModifiedPeriod::LastThirtyDays, 29),
        ] {
            let filters = Filters::default().with_modified(filter_at(period, now));
            let start = today.checked_sub_days(Days::new(preceding)).unwrap();
            assert!(matches_on(&filters, start, 0));
            assert!(!matches_on(&filters, start.pred_opt().unwrap(), 23));
            assert!(matches_on(&filters, today, 23));
            assert!(!matches_on(&filters, today.succ_opt().unwrap(), 0));
        }
    }

    #[test]
    fn missing_midnight_uses_the_first_valid_local_instant() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let first = date.and_hms_opt(0, 30, 0).unwrap();
        let resolve =
            |time: NaiveDateTime| (time >= first).then(|| SystemTime::from(time.and_utc()));
        assert_eq!(day_start(date, &resolve), resolve(first));
    }
}
