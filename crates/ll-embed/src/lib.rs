#![forbid(unsafe_code)]

use ll_core::{DocId, Error, Result, ports::TextEmbedder};
use rusqlite::{Connection, params};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingHit {
    pub document_id: DocId,
    pub page_no: u32,
    pub text: String,
    pub score: f32,
}

pub struct SqliteEmbeddingIndex<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteEmbeddingIndex<'a> {
    pub fn new(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS embeddings (
				document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
				page_no INTEGER NOT NULL,
				model TEXT NOT NULL,
				dimensions INTEGER NOT NULL,
				vector BLOB NOT NULL,
				text TEXT NOT NULL,
				PRIMARY KEY (document_id, page_no, model)
			);
			CREATE INDEX IF NOT EXISTS idx_embeddings_model ON embeddings(model, dimensions);",
        )
        .map_err(Error::storage)?;
        Ok(Self { conn })
    }

    pub fn upsert(
        &self,
        document_id: DocId,
        page_no: u32,
        model: &str,
        vector: &[f32],
        text: &str,
    ) -> Result<()> {
        validate_vector(vector)?;
        if model.trim().is_empty() {
            return Err(Error::Invalid("embedding model name is empty".into()));
        }
        let dimensions = i64::try_from(vector.len())
            .map_err(|_| Error::Invalid("embedding dimensions exceed SQLite range".into()))?;
        self.conn
            .execute(
                "INSERT INTO embeddings (document_id, page_no, model, dimensions, vector, text)
			 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
			 ON CONFLICT(document_id, page_no, model) DO UPDATE SET
				dimensions=excluded.dimensions, vector=excluded.vector, text=excluded.text",
                params![
                    document_id.to_string(),
                    page_no,
                    model,
                    dimensions,
                    encode_vector(vector),
                    text
                ],
            )
            .map_err(Error::storage)?;
        Ok(())
    }

    pub fn search(&self, model: &str, query: &[f32], limit: usize) -> Result<Vec<EmbeddingHit>> {
        validate_vector(query)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
			"SELECT document_id, page_no, vector, text FROM embeddings WHERE model=?1 AND dimensions=?2",
		).map_err(Error::storage)?;
        let rows = statement
            .query_map(params![model, query.len()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(Error::storage)?;

        let mut hits = Vec::new();
        for row in rows {
            let (id, page_no, bytes, text) = row.map_err(Error::storage)?;
            let vector = decode_vector(&bytes)?;
            let Some(score) = cosine_similarity(query, &vector) else {
                continue;
            };
            let document_id =
                DocId::from_str(&id).map_err(|error| Error::Invalid(error.to_string()))?;
            hits.push(EmbeddingHit {
                document_id,
                page_no,
                text,
                score,
            });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then(left.document_id.cmp(&right.document_id))
                .then(left.page_no.cmp(&right.page_no))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

pub fn embed_texts(embedder: &dyn TextEmbedder, texts: &[String]) -> Result<Vec<Vec<f32>>> {
    let vectors = embedder.embed_passages(texts)?;
    if vectors.len() != texts.len() {
        return Err(Error::Inference(format!(
            "embedder returned {} vectors for {} texts",
            vectors.len(),
            texts.len()
        )));
    }
    for vector in &vectors {
        if vector.len() != embedder.dim() || vector.iter().any(|value| !value.is_finite()) {
            return Err(Error::Inference(
                "embedder returned an invalid vector".into(),
            ));
        }
    }
    Ok(vectors)
}

fn validate_vector(vector: &[f32]) -> Result<()> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(Error::Invalid(
            "embedding vector must be non-empty and finite".into(),
        ));
    }
    Ok(())
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_vector(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return Err(Error::Invalid(
            "stored embedding has an invalid byte length".into(),
        ));
    }
    let values = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect::<Vec<_>>();
    validate_vector(&values)?;
    Ok(values)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() {
        return None;
    }
    let (dot, left_norm, right_norm) =
        left.iter()
            .zip(right)
            .fold((0.0f64, 0.0f64, 0.0f64), |(dot, ln, rn), (a, b)| {
                let (a, b) = (*a as f64, *b as f64);
                (dot + a * b, ln + a * a, rn + b * b)
            });
    (left_norm > 0.0 && right_norm > 0.0).then(|| (dot / (left_norm * right_norm).sqrt()) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_store::Store;

    #[test]
    fn stores_and_ranks_vectors_in_sqlite() {
        let store = Store::open_in_memory().unwrap();
        let document = store
            .insert_document("a.pdf", "hash-a", ll_core::DocKind::Invoice)
            .unwrap()
            .0
            .id;
        let other = store
            .insert_document("b.pdf", "hash-b", ll_core::DocKind::Invoice)
            .unwrap()
            .0
            .id;
        let index = SqliteEmbeddingIndex::new(store.conn()).unwrap();
        index
            .upsert(document, 0, "test-model", &[1.0, 0.0], "vendor page")
            .unwrap();
        index
            .upsert(other, 0, "test-model", &[0.0, 1.0], "total page")
            .unwrap();

        let hits = index.search("test-model", &[1.0, 0.0], 1).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, document);
        assert_eq!(hits[0].text, "vendor page");
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rejects_invalid_vectors() {
        let store = Store::open_in_memory().unwrap();
        let document = store
            .insert_document("a.pdf", "hash-a", ll_core::DocKind::Invoice)
            .unwrap()
            .0
            .id;
        let index = SqliteEmbeddingIndex::new(store.conn()).unwrap();
        assert!(index.upsert(document, 0, "m", &[f32::NAN], "bad").is_err());
        assert!(index.search("m", &[], 5).is_err());
    }
}
