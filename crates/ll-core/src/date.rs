use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateParseError;

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if Self::is_leap_year(year) => 29,
            2 => 28,
            _ => return None,
        };
        (1..=days_in_month)
            .contains(&day)
            .then_some(Self { year, month, day })
    }

    pub fn parse(input: &str) -> Result<Self, DateParseError> {
        let parts: Vec<String> = input
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .map(|part| {
                let lower = part.to_ascii_lowercase();
                ["st", "nd", "rd", "th"]
                    .iter()
                    .find_map(|suffix| {
                        lower
                            .strip_suffix(suffix)
                            .filter(|_| part.len() > suffix.len())
                    })
                    .unwrap_or(&lower)
                    .to_string()
            })
            .collect();

        if parts.len() != 3 {
            return Err(DateParseError);
        }

        let month_name = |part: &str| -> Option<u8> {
            [
                "january",
                "february",
                "march",
                "april",
                "may",
                "june",
                "july",
                "august",
                "september",
                "october",
                "november",
                "december",
            ]
            .iter()
            .position(|name| name.starts_with(part) && part.len() >= 3)
            .map(|index| index as u8 + 1)
        };
        let year_number = |part: &str| -> Option<i32> {
            let year = part.parse::<i32>().ok()?;
            Some(if part.len() == 2 {
                if year < 70 { year + 2000 } else { year + 1900 }
            } else {
                year
            })
        };

        if let Some(month) = month_name(&parts[0]) {
            return Self::new(
                year_number(&parts[2]).ok_or(DateParseError)?,
                month,
                parts[1].parse().map_err(|_| DateParseError)?,
            )
            .ok_or(DateParseError);
        }
        if let Some(month) = month_name(&parts[1]) {
            return Self::new(
                year_number(&parts[2]).ok_or(DateParseError)?,
                month,
                parts[0].parse().map_err(|_| DateParseError)?,
            )
            .ok_or(DateParseError);
        }

        if parts[0].len() >= 4 {
            return Self::new(
                parts[0].parse().map_err(|_| DateParseError)?,
                parts[1].parse().map_err(|_| DateParseError)?,
                parts[2].parse().map_err(|_| DateParseError)?,
            )
            .ok_or(DateParseError);
        }

        let first = parts[0].parse::<u8>().map_err(|_| DateParseError)?;
        let second = parts[1].parse::<u8>().map_err(|_| DateParseError)?;
        let year = year_number(&parts[2]).ok_or(DateParseError)?;
        let (month, day) = if first > 12 && second <= 12 {
            (second, first)
        } else {
            (first, second)
        };
        Self::new(year, month, day).ok_or(DateParseError)
    }

    pub fn to_days(self) -> i64 {
        let year = self.year as i64 - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let adjusted_month = self.month as i64 + if self.month > 2 { -3 } else { 9 };
        let day_of_year = (153 * adjusted_month + 2) / 5 + self.day as i64 - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_part = (5 * day_of_year + 2) / 153;
        let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u8;
        let month = if month_part < 10 {
            month_part + 3
        } else {
            month_part - 9
        } as u8;
        year += i64::from(month <= 2);
        Self {
            year: year as i32,
            month,
            day,
        }
    }

    pub fn from_unix_seconds(seconds: i64) -> Self {
        Self::from_days(seconds.div_euclid(86_400))
    }

    fn is_leap_year(year: i32) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_and_parses_supported_dates() {
        assert!(Date::new(2024, 2, 29).is_some());
        assert!(Date::new(2023, 2, 29).is_none());
        for (input, expected) in [
            ("2024-03-05", Date::new(2024, 3, 5).unwrap()),
            ("March 5, 2024", Date::new(2024, 3, 5).unwrap()),
            ("5th March 2024", Date::new(2024, 3, 5).unwrap()),
            ("03/05/2024", Date::new(2024, 3, 5).unwrap()),
            ("25/03/2024", Date::new(2024, 3, 25).unwrap()),
        ] {
            assert_eq!(Date::parse(input), Ok(expected));
        }
    }

    #[test]
    fn day_conversion_roundtrips_before_and_after_epoch() {
        for date in [
            Date::new(1969, 7, 20).unwrap(),
            Date::new(1970, 1, 1).unwrap(),
            Date::new(2000, 2, 29).unwrap(),
            Date::new(2024, 3, 5).unwrap(),
            Date::new(2100, 3, 1).unwrap(),
        ] {
            assert_eq!(Date::from_days(date.to_days()), date);
        }
    }
}
