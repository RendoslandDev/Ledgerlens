//! Turns a typed question into a validated [`SpendQuery`]. Deterministic keyword matching only:
//! the question chooses filters from closed sets (known vendors, known categories, calendar
//! periods), and the numbers always come from SQL over stored records.

use ll_core::{Currency, Date};
use ll_query::{Breakdown, SpendQuery};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// "Which documents need review?"
    ReviewList,
    Spend { query: SpendQuery, scope: String },
}

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december",
];

pub fn parse(question: &str, today: Date, vendors: &[String], categories: &[String]) -> Intent {
    let q = format!(" {} ", question.to_lowercase().replace(['?', '!', ',', '.'], " "));
    let has = |word: &str| q.contains(&format!(" {word} "));

    if has("review") || has("flagged") || q.contains("need checking") {
        return Intent::ReviewList;
    }

    let breakdown = if q.contains("by vendor") || q.contains("per vendor") || has("vendors") || has("suppliers") {
        Breakdown::Vendor
    } else if q.contains("by month") || q.contains("per month") || has("monthly") {
        Breakdown::Month
    } else if q.contains("by category") || q.contains("per category") || has("categories") {
        Breakdown::Category
    } else {
        Breakdown::Total
    };

    let vendor = longest_match(&q, vendors);
    let category = longest_match(&q, categories);
    let currency = Currency::ALL
        .into_iter()
        .find(|currency| has(&currency.code().to_lowercase()))
        .map(|currency| currency.code().to_string());
    let (from, through, period) = period(&q, today);

    let mut scope = String::new();
    match (&category, &vendor) {
        (Some(category), Some(vendor)) => scope.push_str(&format!("{category} spend with {vendor}")),
        (Some(category), None) => scope.push_str(&format!("{category} spend")),
        (None, Some(vendor)) => scope.push_str(&format!("Spend with {vendor}")),
        (None, None) => scope.push_str("Spend"),
    }
    if let Some(period) = period {
        scope.push(' ');
        scope.push_str(&period);
    }
    if let Some(currency) = &currency {
        scope.push_str(&format!(" in {currency}"));
    }

    Intent::Spend {
        query: SpendQuery {
            from,
            through,
            vendor,
            category,
            currency,
            breakdown,
            reviewed_only: true,
            limit: 50,
        },
        scope,
    }
}

/// The longest known name that appears as whole words in the question, in its stored spelling.
fn longest_match(question: &str, names: &[String]) -> Option<String> {
    names
        .iter()
        .filter(|name| {
            let needle = name.trim().to_lowercase();
            needle.len() >= 2 && question.contains(&format!(" {needle} "))
        })
        .max_by_key(|name| name.len())
        .cloned()
}

fn month_end(year: i32, month: u8) -> Date {
    let next = if month == 12 { Date::new(year + 1, 1, 1) } else { Date::new(year, month + 1, 1) };
    next.map(|date| Date::from_days(date.to_days() - 1))
        .unwrap_or(Date { year, month, day: 28 })
}

fn month_range(year: i32, month: u8) -> (Option<Date>, Option<Date>) {
    (Date::new(year, month, 1), Some(month_end(year, month)))
}

fn period(q: &str, today: Date) -> (Option<Date>, Option<Date>, Option<String>) {
    let label = |month: u8, year: i32| format!("in {} {year}", capitalize(MONTHS[usize::from(month) - 1]));
    if q.contains("this month") {
        let (from, through) = month_range(today.year, today.month);
        return (from, through, Some(label(today.month, today.year)));
    }
    if q.contains("last month") {
        let (year, month) = if today.month == 1 { (today.year - 1, 12) } else { (today.year, today.month - 1) };
        let (from, through) = month_range(year, month);
        return (from, through, Some(label(month, year)));
    }
    if q.contains("this year") {
        return (Date::new(today.year, 1, 1), Date::new(today.year, 12, 31), Some(format!("in {}", today.year)));
    }
    if q.contains("last year") {
        let year = today.year - 1;
        return (Date::new(year, 1, 1), Date::new(year, 12, 31), Some(format!("in {year}")));
    }
    if let Some(days) = last_n_days(q) {
        let from = Date::from_days(today.to_days() - i64::from(days) + 1);
        return (Some(from), Some(today), Some(format!("in the last {days} days")));
    }

    let year = q
        .split_whitespace()
        .filter_map(|word| word.parse::<i32>().ok())
        .find(|year| (1990..=2100).contains(year));
    let month = MONTHS.iter().position(|name| {
        q.contains(&format!(" {name} ")) || (name.len() > 3 && q.contains(&format!(" {} ", &name[..3])))
    });
    match (month, year) {
        (Some(index), year) => {
            let month = (index + 1) as u8;
            // A bare month means its most recent occurrence.
            let year = year.unwrap_or(if month > today.month { today.year - 1 } else { today.year });
            let (from, through) = month_range(year, month);
            (from, through, Some(label(month, year)))
        }
        (None, Some(year)) => (Date::new(year, 1, 1), Date::new(year, 12, 31), Some(format!("in {year}"))),
        (None, None) => (None, None, None),
    }
}

fn last_n_days(q: &str) -> Option<u32> {
    let words: Vec<&str> = q.split_whitespace().collect();
    words.windows(3).find_map(|window| match window {
        ["last" | "past", count, "days"] => count.parse().ok().filter(|days| (1..=3660).contains(days)),
        _ => None,
    })
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> Date {
        Date::new(2026, 10, 27).unwrap()
    }

    fn spend(question: &str) -> (SpendQuery, String) {
        let vendors = vec!["Kumasi Cloud Services Ltd".to_string(), "Luma".to_string()];
        let categories = vec!["Software".to_string(), "Travel".to_string()];
        match parse(question, today(), &vendors, &categories) {
            Intent::Spend { query, scope } => (query, scope),
            Intent::ReviewList => panic!("expected a spend question: {question}"),
        }
    }

    #[test]
    fn category_and_period_become_filters() {
        let (query, scope) = spend("How much did we spend on software this month?");
        assert_eq!(query.category.as_deref(), Some("Software"));
        assert_eq!(query.from, Date::new(2026, 10, 1));
        assert_eq!(query.through, Date::new(2026, 10, 31));
        assert_eq!(query.breakdown, Breakdown::Total);
        assert!(query.reviewed_only);
        assert_eq!(scope, "Software spend in October 2026");
    }

    #[test]
    fn vendors_months_and_breakdowns() {
        let (query, _) = spend("spend with kumasi cloud services ltd in september 2026 by month");
        assert_eq!(query.vendor.as_deref(), Some("Kumasi Cloud Services Ltd"));
        assert_eq!(query.from, Date::new(2026, 9, 1));
        assert_eq!(query.through, Date::new(2026, 9, 30));
        assert_eq!(query.breakdown, Breakdown::Month);

        let (query, _) = spend("top vendors last month");
        assert_eq!(query.breakdown, Breakdown::Vendor);
        assert_eq!(query.from, Date::new(2026, 9, 1));

        let (query, _) = spend("spend in december");
        assert_eq!(query.from, Date::new(2025, 12, 1), "a future month means last year's");

        let (query, _) = spend("total in GHS for the last 30 days");
        assert_eq!(query.currency.as_deref(), Some("GHS"));
        assert_eq!(query.from, Date::new(2026, 9, 28));
    }

    #[test]
    fn leap_february_ends_on_the_29th() {
        assert_eq!(month_end(2028, 2), Date::new(2028, 2, 29).unwrap());
        assert_eq!(month_end(2026, 12), Date::new(2026, 12, 31).unwrap());
    }

    #[test]
    fn review_questions_are_recognised_and_text_never_becomes_sql() {
        assert_eq!(parse("Which invoices need review?", today(), &[], &[]), Intent::ReviewList);
        let (query, _) = spend("'; DROP TABLE records; --");
        assert_eq!((query.vendor, query.category), (None, None));
        query_plan_is_valid(&spend("everything").0);
    }

    fn query_plan_is_valid(query: &SpendQuery) {
        assert!(query.plan().is_ok());
    }
}
