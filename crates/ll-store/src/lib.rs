use ll_core::{
    Date, DocId, DocKind, DocStatus, Error, ExtractedField, FieldName, FieldValue, LineItem,
    Result, now_unix,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::{path::Path, str::FromStr};

#[derive(Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

const STORE_MIGRATIONS: &[Migration] = &[
Migration {
    version: 1,
    name: "documents and records",
    sql: "
        CREATE TABLE documents (
            id TEXT PRIMARY KEY,
            sha256 TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            source_path TEXT,
            blob_key TEXT NOT NULL,
            page_count INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE fields (
            document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            value_json TEXT NOT NULL,
            confidence REAL NOT NULL,
            tier TEXT NOT NULL,
            PRIMARY KEY (document_id, name)
        );
        CREATE TABLE line_items (
            document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,
            description TEXT NOT NULL,
            quantity REAL NOT NULL,
            unit_minor INTEGER NOT NULL,
            unit_currency TEXT NOT NULL,
            amount_minor INTEGER NOT NULL,
            amount_currency TEXT NOT NULL,
            PRIMARY KEY (document_id, position)
        );
        CREATE TABLE records (
            document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
            vendor TEXT,
            invoice_date TEXT,
            currency TEXT,
            total_minor INTEGER
        );
        CREATE INDEX idx_documents_status ON documents(status, updated_at);
        CREATE INDEX idx_records_date ON records(invoice_date);
        CREATE INDEX idx_records_vendor ON records(vendor);
    ",
}, Migration {
    version: 2,
    name: "record category and invoice number",
    sql: "
        ALTER TABLE records ADD COLUMN category TEXT;
        ALTER TABLE records ADD COLUMN invoice_number TEXT;
        CREATE INDEX idx_records_category ON records(category);
    ",
}, Migration {
    version: 3,
    name: "field corrections",
    sql: "
        CREATE TABLE corrections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            old_value_json TEXT,
            old_tier TEXT,
            old_confidence REAL,
            new_value_json TEXT,
            changed_at INTEGER NOT NULL
        );
        CREATE INDEX idx_corrections_document ON corrections(document_id, id);
    ",
}];

pub fn run_migrations(
    conn: &mut Connection,
    namespace: &str,
    migrations: &[Migration],
) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _ll_migrations (
            namespace TEXT NOT NULL,
            version INTEGER NOT NULL,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            PRIMARY KEY (namespace, version)
        );",
    )
    .map_err(Error::storage)?;

    for migration in migrations {
        let applied = conn
            .query_row(
                "SELECT 1 FROM _ll_migrations WHERE namespace=?1 AND version=?2",
                params![namespace, migration.version],
                |_| Ok(()),
            )
            .optional()
            .map_err(Error::storage)?
            .is_some();
        if applied {
            continue;
        }

        let transaction = conn.transaction().map_err(Error::storage)?;
        apply_migration(&transaction, namespace, migration)?;
        transaction.commit().map_err(Error::storage)?;
    }
    Ok(())
}

fn apply_migration(
    transaction: &Transaction<'_>,
    namespace: &str,
    migration: &Migration,
) -> Result<()> {
    transaction
        .execute_batch(migration.sql)
        .map_err(Error::storage)?;
    transaction
        .execute(
            "INSERT INTO _ll_migrations (namespace, version, name, applied_at) VALUES (?1, ?2, ?3, ?4)",
            params![namespace, migration.version, migration.name, now_unix()],
        )
        .map_err(Error::storage)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: DocId,
    pub sha256: String,
    pub source_path: Option<String>,
    pub kind: DocKind,
    pub status: DocStatus,
}

/// One change a person made to a field: what the machine (or an earlier edit) said, and what it
/// became. Kept as data so corrections can feed evaluation and calibration (ADR 0002).
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub field: FieldName,
    pub old: Option<FieldValue>,
    pub old_tier: Option<ll_core::Tier>,
    pub old_confidence: Option<f32>,
    pub new: Option<FieldValue>,
    pub changed_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvoiceRow {
    pub document_id: DocId,
    pub vendor: Option<String>,
    pub invoice_date: Option<Date>,
    pub currency: Option<String>,
    pub total_minor: Option<i64>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut store = Self {
            conn: Connection::open(path).map_err(Error::storage)?,
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut store = Self {
            conn: Connection::open_in_memory().map_err(Error::storage)?,
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&mut self) -> Result<()> {
        self.conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(Error::storage)?;
        run_migrations(&mut self.conn, "store", STORE_MIGRATIONS)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn insert_document(
        &self,
        source_path: &str,
        sha256: &str,
        kind: DocKind,
    ) -> Result<(Document, bool)> {
        if let Some(existing) = self
            .conn
            .query_row(
                "SELECT id, sha256, source_path, kind, status FROM documents WHERE sha256=?1",
                [sha256],
                read_document,
            )
            .optional()
            .map_err(Error::storage)?
        {
            return Ok((existing, false));
        }

        let id = DocId::new();
        let now = now_unix();
        self.conn
            .execute(
                "INSERT INTO documents (id, sha256, kind, source_path, blob_key, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?2, ?5, ?6, ?6)",
                params![id.to_string(), sha256, kind.as_str(), source_path, DocStatus::Queued.as_str(), now],
            )
            .map_err(Error::storage)?;
        self.get_document(id).map(|document| (document, true))
    }

    pub fn get_document(&self, id: DocId) -> Result<Document> {
        self.conn
            .query_row(
                "SELECT id, sha256, source_path, kind, status FROM documents WHERE id=?1",
                [id.to_string()],
                read_document,
            )
            .optional()
            .map_err(Error::storage)?
            .ok_or_else(|| Error::NotFound(format!("document {id}")))
    }

    pub fn set_kind(&self, id: DocId, kind: DocKind) -> Result<()> {
        self.update_document(id, "kind", kind.as_str())
    }

    pub fn set_status(&self, id: DocId, status: DocStatus) -> Result<()> {
        self.update_document(id, "status", status.as_str())
    }

    fn update_document(&self, id: DocId, column: &str, value: &str) -> Result<()> {
        let sql = match column {
            "kind" => "UPDATE documents SET kind=?2, updated_at=?3 WHERE id=?1",
            "status" => "UPDATE documents SET status=?2, updated_at=?3 WHERE id=?1",
            _ => return Err(Error::Invalid("unsupported document update".into())),
        };
        let changed = self
            .conn
            .execute(sql, params![id.to_string(), value, now_unix()])
            .map_err(Error::storage)?;
        if changed == 0 {
            return Err(Error::NotFound(format!("document {id}")));
        }
        Ok(())
    }

    pub fn upsert_field(&self, id: DocId, field: &ExtractedField) -> Result<()> {
        let value = serde_json::to_string(&field.value).map_err(Error::storage)?;
        self.conn
            .execute(
                "INSERT INTO fields (document_id, name, value_json, confidence, tier) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(document_id, name) DO UPDATE SET value_json=excluded.value_json,
                    confidence=excluded.confidence, tier=excluded.tier",
                params![id.to_string(), field.name.as_str(), value, field.confidence.get(), field.tier.as_str()],
            )
            .map_err(Error::storage)?;
        Ok(())
    }

    /// Records a person's change to `name`. `old` is the stored field before the change.
    pub fn record_correction(
        &self,
        id: DocId,
        name: FieldName,
        old: Option<&ExtractedField>,
        new: Option<&FieldValue>,
    ) -> Result<()> {
        let old_value = old.map(|f| serde_json::to_string(&f.value)).transpose().map_err(Error::storage)?;
        let new_value = new.map(serde_json::to_string).transpose().map_err(Error::storage)?;
        self.conn
            .execute(
                "INSERT INTO corrections (document_id, name, old_value_json, old_tier, old_confidence, new_value_json, changed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.to_string(),
                    name.as_str(),
                    old_value,
                    old.map(|f| f.tier.as_str()),
                    old.map(|f| f.confidence.get()),
                    new_value,
                    now_unix()
                ],
            )
            .map_err(Error::storage)?;
        Ok(())
    }

    /// Corrections for one document, oldest first.
    pub fn corrections(&self, id: DocId) -> Result<Vec<Correction>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT name, old_value_json, old_tier, old_confidence, new_value_json, changed_at
                 FROM corrections WHERE document_id=?1 ORDER BY id",
            )
            .map_err(Error::storage)?;
        let rows = statement
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f32>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(Error::storage)?;
        let value = |json: Option<String>| -> Result<Option<FieldValue>> {
            json.map(|json| serde_json::from_str(&json).map_err(Error::storage)).transpose()
        };
        rows.map(|row| {
            let (name, old, tier, confidence, new, changed_at) = row.map_err(Error::storage)?;
            Ok(Correction {
                field: FieldName::from_str(&name)?,
                old: value(old)?,
                old_tier: tier.map(|tier| ll_core::Tier::from_str(&tier)).transpose()?,
                old_confidence: confidence,
                new: value(new)?,
                changed_at,
            })
        })
        .collect()
    }

    /// Removes one field; returns whether it existed.
    pub fn delete_field(&self, id: DocId, name: FieldName) -> Result<bool> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM fields WHERE document_id=?1 AND name=?2",
                params![id.to_string(), name.as_str()],
            )
            .map_err(Error::storage)?;
        Ok(removed > 0)
    }

    /// Deletes a document and, through foreign keys, everything derived from it.
    pub fn delete_document(&self, id: DocId) -> Result<()> {
        let removed = self
            .conn
            .execute("DELETE FROM documents WHERE id=?1", [id.to_string()])
            .map_err(Error::storage)?;
        if removed == 0 {
            return Err(Error::NotFound(format!("document {id}")));
        }
        Ok(())
    }

    /// Every document, newest first.
    pub fn documents(&self) -> Result<Vec<Document>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, sha256, source_path, kind, status FROM documents ORDER BY created_at DESC, id DESC")
            .map_err(Error::storage)?;
        let rows = statement.query_map([], read_document).map_err(Error::storage)?;
        rows.map(|row| row.map_err(Error::storage)).collect()
    }

    pub fn fields(&self, id: DocId) -> Result<Vec<ExtractedField>> {
        let mut statement = self
            .conn
            .prepare("SELECT name, value_json, confidence, tier FROM fields WHERE document_id=?1 ORDER BY name")
            .map_err(Error::storage)?;
        let rows = statement
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f32>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(Error::storage)?;
        rows.map(|row| {
            let (name, value, confidence, tier) = row.map_err(Error::storage)?;
            Ok(ExtractedField {
                name: FieldName::from_str(&name)?,
                value: serde_json::from_str::<FieldValue>(&value).map_err(Error::storage)?,
                confidence: ll_core::Confidence::new(confidence),
                tier: ll_core::Tier::from_str(&tier)?,
            })
        })
        .collect()
    }

    pub fn replace_line_items(&self, id: DocId, items: &[LineItem]) -> Result<()> {
        let transaction = self.conn.unchecked_transaction().map_err(Error::storage)?;
        transaction
            .execute(
                "DELETE FROM line_items WHERE document_id=?1",
                [id.to_string()],
            )
            .map_err(Error::storage)?;
        for (position, item) in items.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO line_items (document_id, position, description, quantity, unit_minor, unit_currency, amount_minor, amount_currency)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id.to_string(), position, item.description, item.quantity, item.unit_price.minor,
                        item.unit_price.currency.code(), item.amount.minor, item.amount.currency.code()],
                )
                .map_err(Error::storage)?;
        }
        transaction.commit().map_err(Error::storage)
    }

    pub fn line_items(&self, id: DocId) -> Result<Vec<LineItem>> {
        let mut statement = self.conn.prepare(
            "SELECT description, quantity, unit_minor, unit_currency, amount_minor, amount_currency
             FROM line_items WHERE document_id=?1 ORDER BY position",
        ).map_err(Error::storage)?;
        let rows = statement
            .query_map([id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(Error::storage)?;
        rows.map(|row| {
            let (description, quantity, unit_minor, unit_currency, amount_minor, amount_currency) =
                row.map_err(Error::storage)?;
            Ok(LineItem {
                description,
                quantity,
                unit_price: ll_core::Money::new(
                    unit_minor,
                    ll_core::Currency::new(&unit_currency)?,
                ),
                amount: ll_core::Money::new(
                    amount_minor,
                    ll_core::Currency::new(&amount_currency)?,
                ),
            })
        })
        .collect()
    }

    pub fn refresh_invoice(&self, id: DocId) -> Result<()> {
        let document = self.get_document(id)?;
        if !matches!(document.kind, DocKind::Invoice | DocKind::Receipt) {
            self.conn
                .execute("DELETE FROM records WHERE document_id=?1", [id.to_string()])
                .map_err(Error::storage)?;
            return Ok(());
        }

        let mut vendor = None;
        let mut invoice_date = None;
        let mut currency = None;
        let mut total_minor = None;
        let mut category = None;
        let mut invoice_number = None;
        for field in self.fields(id)? {
            match (field.name, field.value) {
                (FieldName::Vendor, FieldValue::Text(value)) => vendor = Some(value),
                (FieldName::InvoiceDate, FieldValue::Date(value)) => {
                    invoice_date = Some(value.to_string())
                }
                (FieldName::Currency, FieldValue::Text(value)) => currency = Some(value),
                (FieldName::Category, FieldValue::Text(value)) => category = Some(value),
                (FieldName::InvoiceNumber, FieldValue::Text(value)) => invoice_number = Some(value),
                (FieldName::Total, FieldValue::Money(value)) => {
                    total_minor = Some(value.minor);
                    currency.get_or_insert_with(|| value.currency.code().to_string());
                }
                _ => {}
            }
        }
        self.conn.execute(
            "INSERT INTO records (document_id, vendor, invoice_date, currency, total_minor, category, invoice_number)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(document_id) DO UPDATE SET vendor=excluded.vendor, invoice_date=excluded.invoice_date,
                currency=excluded.currency, total_minor=excluded.total_minor, category=excluded.category,
                invoice_number=excluded.invoice_number",
            params![id.to_string(), vendor, invoice_date, currency, total_minor, category, invoice_number],
        ).map_err(Error::storage)?;
        Ok(())
    }

    pub fn invoices(&self) -> Result<Vec<InvoiceRow>> {
        let mut statement = self.conn.prepare(
            "SELECT document_id, vendor, invoice_date, currency, total_minor FROM records ORDER BY document_id",
        ).map_err(Error::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .map_err(Error::storage)?;
        rows.map(|row| {
            let (id, vendor, date, currency, total_minor) = row.map_err(Error::storage)?;
            Ok(InvoiceRow {
                document_id: DocId::from_str(&id)
                    .map_err(|error| Error::Invalid(error.to_string()))?,
                vendor,
                invoice_date: date
                    .map(|value| Date::parse(&value))
                    .transpose()
                    .map_err(|_| Error::Invalid("invalid stored invoice date".into()))?,
                currency,
                total_minor,
            })
        })
        .collect()
    }
}

fn read_document(row: &rusqlite::Row<'_>) -> rusqlite::Result<Document> {
    let id = row.get::<_, String>(0)?;
    let kind = row.get::<_, String>(3)?;
    let status = row.get::<_, String>(4)?;
    let id = DocId::from_str(&id).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.to_string())),
        )
    })?;
    let kind = DocKind::from_str(&kind).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.to_string())),
        )
    })?;
    let status = DocStatus::from_str(&status).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(error.to_string())),
        )
    })?;
    Ok(Document {
        id,
        sha256: row.get(1)?,
        source_path: row.get(2)?,
        kind,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_core::{Confidence, Currency, FieldValue, Money, Tier};

    #[test]
    fn document_insert_deduplicates_by_content_hash() {
        let store = Store::open_in_memory().unwrap();
        let (first, inserted) = store
            .insert_document("first.pdf", "same-hash", DocKind::Invoice)
            .unwrap();
        let (second, inserted_again) = store
            .insert_document("second.pdf", "same-hash", DocKind::Receipt)
            .unwrap();
        assert!(inserted);
        assert!(!inserted_again);
        assert_eq!(first.id, second.id);
        assert_eq!(second.source_path.as_deref(), Some("first.pdf"));
        assert_eq!(second.kind, DocKind::Invoice);
    }

    #[test]
    fn field_line_item_and_invoice_projection_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let (document, _) = store
            .insert_document("invoice.pdf", "invoice-hash", DocKind::Invoice)
            .unwrap();
        let vendor = ExtractedField {
            name: FieldName::Vendor,
            value: FieldValue::Text("Acme".into()),
            confidence: Confidence::new(0.95),
            tier: Tier::Rules,
        };
        let total = ExtractedField {
            name: FieldName::Total,
            value: FieldValue::Money(Money::new(12345, Currency::USD)),
            confidence: Confidence::ONE,
            tier: Tier::Rules,
        };
        let date = ExtractedField {
            name: FieldName::InvoiceDate,
            value: FieldValue::Date(Date::new(2025, 4, 3).unwrap()),
            confidence: Confidence::new(0.9),
            tier: Tier::Rules,
        };
        for field in [&vendor, &total, &date] {
            store.upsert_field(document.id, field).unwrap();
        }
        assert_eq!(store.fields(document.id).unwrap().len(), 3);

        let item = LineItem {
            description: "Service".into(),
            quantity: 1.0,
            unit_price: Money::new(12345, Currency::USD),
            amount: Money::new(12345, Currency::USD),
        };
        store
            .replace_line_items(document.id, std::slice::from_ref(&item))
            .unwrap();
        assert_eq!(store.line_items(document.id).unwrap(), [item]);

        store.refresh_invoice(document.id).unwrap();
        let invoices = store.invoices().unwrap();
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].vendor.as_deref(), Some("Acme"));
        assert_eq!(
            invoices[0].invoice_date,
            Some(Date::new(2025, 4, 3).unwrap())
        );
        assert_eq!(invoices[0].currency.as_deref(), Some("USD"));
        assert_eq!(invoices[0].total_minor, Some(12345));
    }

    #[test]
    fn projection_carries_category_and_invoice_number_and_delete_cascades() {
        let store = Store::open_in_memory().unwrap();
        let (document, _) = store
            .insert_document("receipt.pdf", "receipt-hash", DocKind::Receipt)
            .unwrap();
        for (name, value) in [
            (FieldName::Category, "Travel"),
            (FieldName::InvoiceNumber, "R-77"),
        ] {
            store
                .upsert_field(
                    document.id,
                    &ExtractedField {
                        name,
                        value: FieldValue::Text(value.into()),
                        confidence: Confidence::ONE,
                        tier: Tier::Human,
                    },
                )
                .unwrap();
        }
        store.refresh_invoice(document.id).unwrap();
        let (category, number): (String, String) = store
            .conn()
            .query_row(
                "SELECT category, invoice_number FROM records WHERE document_id=?1",
                [document.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((category.as_str(), number.as_str()), ("Travel", "R-77"));

        let before = store.fields(document.id).unwrap().into_iter().find(|f| f.name == FieldName::Category);
        store
            .record_correction(document.id, FieldName::Category, before.as_ref(), Some(&FieldValue::Text("Meals".into())))
            .unwrap();
        store.record_correction(document.id, FieldName::Vendor, None, None).unwrap();
        let history = store.corrections(document.id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].old, Some(FieldValue::Text("Travel".into())));
        assert_eq!(history[0].old_tier, Some(Tier::Human));
        assert_eq!(history[0].new, Some(FieldValue::Text("Meals".into())));
        assert_eq!((history[1].old.as_ref(), history[1].new.as_ref()), (None, None));

        assert!(store.delete_field(document.id, FieldName::Category).unwrap());
        assert!(!store.delete_field(document.id, FieldName::Category).unwrap());
        assert_eq!(store.documents().unwrap().len(), 1);
        store.delete_document(document.id).unwrap();
        assert!(store.documents().unwrap().is_empty());
        assert!(store.fields(document.id).unwrap().is_empty());
        assert!(store.invoices().unwrap().is_empty());
        assert!(store.corrections(document.id).unwrap().is_empty(), "history goes with the document");
        assert!(store.delete_document(document.id).is_err());
    }

    #[test]
    fn migrations_are_applied_once_per_namespace_and_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migration = Migration {
            version: 1,
            name: "probe",
            sql: "CREATE TABLE migration_probe (value INTEGER);",
        };
        run_migrations(&mut conn, "test", &[migration]).unwrap();
        run_migrations(&mut conn, "test", &[migration]).unwrap();
        let applied: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM _ll_migrations WHERE namespace='test' AND version=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }
}
