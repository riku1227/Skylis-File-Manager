use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DateGroup {
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    Earlier,
    Unknown,
}

impl DateGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "今日",
            Self::Yesterday => "昨日",
            Self::ThisWeek => "今週",
            Self::LastWeek => "先週",
            Self::ThisMonth => "今月",
            Self::LastMonth => "先月",
            Self::Earlier => "それ以前",
            Self::Unknown => "日付なし",
        }
    }
}

pub fn classify(date: Option<NaiveDate>, today: NaiveDate) -> DateGroup {
    let Some(date) = date else {
        return DateGroup::Unknown;
    };
    if date >= today {
        return DateGroup::Today;
    }
    if date == today - Duration::days(1) {
        return DateGroup::Yesterday;
    }
    let week = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    if date >= week {
        return DateGroup::ThisWeek;
    }
    if date >= week - Duration::days(7) {
        return DateGroup::LastWeek;
    }
    let month = today.with_day(1).unwrap();
    if date >= month {
        return DateGroup::ThisMonth;
    }
    let previous = (month - Duration::days(1)).with_day(1).unwrap();
    if date >= previous {
        return DateGroup::LastMonth;
    }
    DateGroup::Earlier
}

#[cfg(test)]
mod tests {
    use super::*;
    fn d(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }
    #[test]
    fn calendar_buckets_are_nonoverlapping_and_cover_missing_and_future() {
        let now = d(2026, 11, 15);
        for (date, expected) in [
            (Some(d(2026, 11, 16)), DateGroup::Today),
            (Some(now), DateGroup::Today),
            (Some(d(2026, 11, 14)), DateGroup::Yesterday),
            (Some(d(2026, 11, 9)), DateGroup::ThisWeek),
            (Some(d(2026, 11, 8)), DateGroup::LastWeek),
            (Some(d(2026, 11, 1)), DateGroup::ThisMonth),
            (Some(d(2026, 10, 31)), DateGroup::LastMonth),
            (Some(d(2026, 9, 30)), DateGroup::Earlier),
            (None, DateGroup::Unknown),
        ] {
            assert_eq!(classify(date, now), expected);
        }
    }
    #[test]
    fn monday_new_year_and_leap_month_boundaries() {
        assert_eq!(
            classify(Some(d(2026, 11, 8)), d(2026, 11, 9)),
            DateGroup::Yesterday
        );
        assert_eq!(
            classify(Some(d(2025, 12, 15)), d(2026, 1, 1)),
            DateGroup::LastMonth
        );
        assert_eq!(
            classify(Some(d(2024, 2, 29)), d(2024, 3, 15)),
            DateGroup::LastMonth
        );
    }
}
