#![forbid(unsafe_code)]

use ll_core::{Currency, Date, Error, Result};
use rusqlite::{Connection, params_from_iter, types::Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakdown {
    Total,
    Month,
    Vendor,
    Category,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendQuery {
    pub from: Option<Date>,
    pub through: Option<Date>,
    pub vendor: Option<String>,
    pub category: Option<String>,
    pub currency: Option<String>,
    pub breakdown: Breakdown,
    /// Only count documents whose extraction validated or that a person approved.
    pub reviewed_only: bool,
    pub limit: u32,
}

/// Opaque validated SQL. Construct plans with [`SpendQuery::plan`].
///
/// ```compile_fail
/// use ll_query::SqlPlan;
/// let _plan = SqlPlan { sql: String::new(), parameters: Vec::new() };
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SqlPlan {
    sql: String,
    parameters: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendRow {
    pub label: Option<String>,
    pub currency: String,
    pub total_minor: i64,
    pub count: u32,
}

impl SpendQuery {
    pub fn plan(&self) -> Result<SqlPlan> {
        if self.limit == 0 || self.limit > 1000 {
            return Err(Error::Invalid(
                "query limit must be between 1 and 1000".into(),
            ));
        }
        if self
            .from
            .zip(self.through)
            .is_some_and(|(from, through)| from > through)
        {
            return Err(Error::Invalid("query start date is after end date".into()));
        }
        if self
            .vendor
            .as_ref()
            .is_some_and(|vendor| vendor.trim().is_empty())
        {
            return Err(Error::Invalid("vendor filter must not be blank".into()));
        }
        if self
            .category
            .as_ref()
            .is_some_and(|category| category.trim().is_empty())
        {
            return Err(Error::Invalid("category filter must not be blank".into()));
        }
        let currency = self.currency.as_deref().map(Currency::new).transpose()?;

        let (label, grouping) = match self.breakdown {
            Breakdown::Total => ("NULL", " GROUP BY currency"),
            Breakdown::Month => (
                "substr(invoice_date, 1, 7)",
                " GROUP BY substr(invoice_date, 1, 7), currency",
            ),
            Breakdown::Vendor => ("vendor", " GROUP BY vendor, currency"),
            Breakdown::Category => ("category", " GROUP BY category, currency"),
        };
        let mut sql = format!(
            "SELECT {label} AS label, currency, SUM(total_minor) AS total_minor, COUNT(*) AS count FROM records WHERE total_minor IS NOT NULL AND currency IS NOT NULL"
        );
        let mut parameters = Vec::new();
        if self.reviewed_only {
            sql.push_str(
                " AND document_id IN (SELECT id FROM documents WHERE status IN ('extracted', 'approved'))",
            );
        }
        if let Some(from) = self.from {
            parameters.push(Value::Text(from.to_string()));
            sql.push_str(&format!(" AND invoice_date >= ?{}", parameters.len()));
        }
        if let Some(through) = self.through {
            parameters.push(Value::Text(through.to_string()));
            sql.push_str(&format!(" AND invoice_date <= ?{}", parameters.len()));
        }
        if let Some(vendor) = &self.vendor {
            parameters.push(Value::Text(vendor.clone()));
            sql.push_str(&format!(" AND vendor = ?{} COLLATE NOCASE", parameters.len()));
        }
        if let Some(category) = &self.category {
            parameters.push(Value::Text(category.clone()));
            sql.push_str(&format!(" AND category = ?{} COLLATE NOCASE", parameters.len()));
        }
        if let Some(currency) = currency {
            parameters.push(Value::Text(currency.code().to_string()));
            sql.push_str(&format!(" AND currency = ?{}", parameters.len()));
        }
        sql.push_str(grouping);
        sql.push_str(" ORDER BY total_minor DESC");
        parameters.push(Value::Integer(i64::from(self.limit)));
        sql.push_str(&format!(" LIMIT ?{}", parameters.len()));
        Ok(SqlPlan { sql, parameters })
    }
}

impl SqlPlan {
    /// The parameterised SQL text, for display. Values are never interpolated into it.
    pub fn sql(&self) -> &str {
        &self.sql
    }
}

pub fn execute(conn: &Connection, plan: &SqlPlan) -> Result<Vec<SpendRow>> {
    let mut statement = conn.prepare(&plan.sql).map_err(Error::storage)?;
    let rows = statement
        .query_map(params_from_iter(plan.parameters.iter()), |row| {
            Ok(SpendRow {
                label: row.get(0)?,
                currency: row.get(1)?,
                total_minor: row.get(2)?,
                count: row.get(3)?,
            })
        })
        .map_err(Error::storage)?;
    rows.map(|row| row.map_err(Error::storage)).collect()
}

pub fn reciprocal_rank_fusion(rankings: &[Vec<String>], rank_constant: u32) -> Vec<(String, f64)> {
    let mut scores = HashMap::<String, f64>::new();
    for ranking in rankings {
        let mut seen = HashSet::new();
        for (rank, id) in ranking.iter().enumerate() {
            if seen.insert(id) {
                *scores.entry(id.clone()).or_default() +=
                    1.0 / (f64::from(rank_constant) + rank as f64 + 1.0);
            }
        }
    }
    let mut results: Vec<_> = scores.into_iter().collect();
    results.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_store::Store;

    fn query() -> SpendQuery {
        SpendQuery {
            from: None,
            through: None,
            vendor: None,
            category: None,
            currency: None,
            breakdown: Breakdown::Total,
            reviewed_only: false,
            limit: 50,
        }
    }

    #[test]
    fn user_text_is_always_a_bound_parameter() {
        let mut request = query();
        request.vendor = Some("x' OR 1=1 --".into());
        let plan = request.plan().unwrap();
        assert!(!plan.sql.contains("x' OR 1=1"));
        assert!(
            plan.parameters
                .contains(&Value::Text("x' OR 1=1 --".into()))
        );
    }

    #[test]
    fn planned_query_executes_against_the_shared_store_schema() {
        let store = Store::open_in_memory().unwrap();
        let document = store
            .insert_document("a.pdf", "hash-a", ll_core::DocKind::Invoice)
            .unwrap()
            .0
            .id;
        store
            .upsert_field(
                document,
                &ll_core::ExtractedField {
                    name: ll_core::FieldName::Vendor,
                    value: ll_core::FieldValue::Text("Acme".into()),
                    confidence: ll_core::Confidence::ONE,
                    tier: ll_core::Tier::Rules,
                },
            )
            .unwrap();
        store.set_kind(document, ll_core::DocKind::Invoice).unwrap();
        store.refresh_invoice(document).unwrap();
        store
            .upsert_field(
                document,
                &ll_core::ExtractedField {
                    name: ll_core::FieldName::Total,
                    value: ll_core::FieldValue::Money(ll_core::Money::new(
                        1234,
                        ll_core::Currency::USD,
                    )),
                    confidence: ll_core::Confidence::ONE,
                    tier: ll_core::Tier::Rules,
                },
            )
            .unwrap();
        store.refresh_invoice(document).unwrap();
        let rows = execute(store.conn(), &query().plan().unwrap()).unwrap();
        assert_eq!(
            rows,
            [SpendRow {
                label: None,
                currency: "USD".into(),
                total_minor: 1234,
                count: 1
            }]
        );
    }

    #[test]
    fn total_spend_never_adds_different_currencies() {
        let store = Store::open_in_memory().unwrap();
        for (path, hash, currency, minor) in [
            ("usd.pdf", "usd-hash", ll_core::Currency::USD, 10000),
            ("eur.pdf", "eur-hash", ll_core::Currency::EUR, 90000),
        ] {
            let document = store
                .insert_document(path, hash, ll_core::DocKind::Invoice)
                .unwrap()
                .0
                .id;
            store
                .upsert_field(
                    document,
                    &ll_core::ExtractedField {
                        name: ll_core::FieldName::Total,
                        value: ll_core::FieldValue::Money(ll_core::Money::new(minor, currency)),
                        confidence: ll_core::Confidence::ONE,
                        tier: ll_core::Tier::Rules,
                    },
                )
                .unwrap();
            store.refresh_invoice(document).unwrap();
        }
        let rows = execute(store.conn(), &query().plan().unwrap()).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .any(|row| row.currency == "USD" && row.total_minor == 10000)
        );
        assert!(
            rows.iter()
                .any(|row| row.currency == "EUR" && row.total_minor == 90000)
        );
    }

    #[test]
    fn category_filter_and_breakdown_are_bound_and_grouped() {
        let store = Store::open_in_memory().unwrap();
        for (hash, category, minor) in [("a", "Travel", 500), ("b", "Travel", 700), ("c", "Meals", 90)] {
            let document = store
                .insert_document(hash, hash, ll_core::DocKind::Receipt)
                .unwrap()
                .0
                .id;
            for (name, value) in [
                (ll_core::FieldName::Category, ll_core::FieldValue::Text(category.into())),
                (
                    ll_core::FieldName::Total,
                    ll_core::FieldValue::Money(ll_core::Money::new(minor, ll_core::Currency::USD)),
                ),
            ] {
                store
                    .upsert_field(
                        document,
                        &ll_core::ExtractedField {
                            name,
                            value,
                            confidence: ll_core::Confidence::ONE,
                            tier: ll_core::Tier::Human,
                        },
                    )
                    .unwrap();
            }
            store.refresh_invoice(document).unwrap();
        }

        let mut filtered = query();
        filtered.category = Some("travel".into());
        let plan = filtered.plan().unwrap();
        assert!(!plan.sql().contains("travel"));
        let rows = execute(store.conn(), &plan).unwrap();
        assert_eq!((rows[0].total_minor, rows[0].count), (1200, 2));

        let mut grouped = query();
        grouped.breakdown = Breakdown::Category;
        let rows = execute(store.conn(), &grouped.plan().unwrap()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label.as_deref(), Some("Travel"));

        let mut reviewed = query();
        reviewed.reviewed_only = true;
        assert!(execute(store.conn(), &reviewed.plan().unwrap()).unwrap().is_empty());
        let first = store.documents().unwrap()[0].id;
        store.set_status(first, ll_core::DocStatus::Approved).unwrap();
        assert_eq!(execute(store.conn(), &reviewed.plan().unwrap()).unwrap().len(), 1);

        let mut blank = query();
        blank.category = Some("  ".into());
        assert!(blank.plan().is_err());
    }

    #[test]
    fn rrf_combines_lists_without_double_counting_a_list() {
        let result = reciprocal_rank_fusion(
            &[
                vec!["a".into(), "b".into(), "a".into()],
                vec!["a".into(), "c".into()],
            ],
            60,
        );
        assert_eq!(result[0].0, "a");
        assert!(result[0].1 > result[1].1);
    }

    #[test]
    fn rejects_invalid_limits_and_ranges() {
        let mut invalid = query();
        invalid.limit = 0;
        assert!(invalid.plan().is_err());
        invalid = query();
        invalid.from = Some(Date::new(2025, 1, 1).unwrap());
        invalid.through = Some(Date::new(2024, 1, 1).unwrap());
        assert!(invalid.plan().is_err());
    }
}
