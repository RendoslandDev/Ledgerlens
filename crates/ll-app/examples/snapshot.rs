//! Prints the service's responses for a data directory as one JSON object keyed by command name,
//! for checking UI rendering against real backend output: `snapshot -- <data-dir>`.
use serde_json::{json, Value};
use std::str::FromStr;

fn main() -> ll_core::Result<()> {
    let dir = std::env::args().nth(1).expect("usage: snapshot <data-dir>");
    let app = ll_app::App::open(dir.as_ref())?;
    let list = app.list()?;
    let mut details = serde_json::Map::new();
    for doc in &list {
        let detail = app.detail(ll_core::DocId::from_str(&doc.id).map_err(|e| ll_core::Error::Invalid(e.to_string()))?)?;
        details.insert(doc.id.clone(), serde_json::to_value(detail).expect("serializable"));
    }
    let snapshot: Value = json!({
        "list_documents": list,
        "get_document": details,
        "report": app.report()?,
        "vendors": app.vendors()?,
        "get_settings": app.settings()?,
        "system_info": app.system_info()?,
        "categories": ll_app::CATEGORIES,
        "search": app.search(&std::env::args().nth(2).unwrap_or_else(|| "copilot".into()))?,
        "ask": app.ask("spend by vendor this year")?,
        "export_plan": app.export_plan(ll_app::ExportFormat::Canonical, std::path::Path::new("/Users/you/Downloads"))?,
    });
    println!("{snapshot}");
    Ok(())
}
