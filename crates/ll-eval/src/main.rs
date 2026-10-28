use ll_core::{Error, FieldName, Result};
use ll_eval::{
    cord::{hundredths, read_cord_parquet},
    score::{Prediction, Routing, Scorecard},
    DatasetInventory, DatasetKind, load_cord, load_sroie,
};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    process,
    str::FromStr,
    time::Instant,
};

const USAGE: &str = "usage:
  ll-eval <sroie|cord> <dataset-directory>
      Print an inventory of labelled samples.
  ll-eval cord-run <cord-parquet> [--limit N] [--details FILE] [--record RESULTS_JSON] [--note TEXT]
      Import every receipt through the Ledgerlens service (text layer or Apple Vision OCR, then
      the rules), score total/subtotal/tax/line prices, and print the run as JSON. --record
      appends the run to a results file; --details writes one JSON line per receipt (scores and OCR text).";

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("cord-run") => cord_run(&args[1..]),
        Some(dataset) if args.len() == 2 => inventory(dataset, PathBuf::from(&args[1])),
        _ => Err(Error::Invalid(USAGE.into())),
    }
}

fn inventory(dataset: &str, root: PathBuf) -> Result<()> {
    let (kind, samples) = match dataset.to_ascii_lowercase().as_str() {
        "sroie" => (DatasetKind::Sroie, load_sroie(root)?),
        "cord" => (DatasetKind::Cord, load_cord(root)?),
        _ => return Err(Error::Invalid(format!("unsupported dataset: {dataset}\n{USAGE}"))),
    };
    let inventory = DatasetInventory { schema_version: 1, dataset: kind, sample_count: samples.len() };
    println!("{}", serde_json::to_string_pretty(&inventory).map_err(Error::storage)?);
    Ok(())
}

struct RunOptions {
    parquet: PathBuf,
    limit: Option<usize>,
    details: Option<PathBuf>,
    record: Option<PathBuf>,
    note: Option<String>,
}

fn parse_options(args: &[String]) -> Result<RunOptions> {
    let mut options = RunOptions { parquet: PathBuf::new(), limit: None, details: None, record: None, note: None };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = || rest.next().cloned().ok_or_else(|| Error::Invalid(format!("{arg} needs a value\n{USAGE}")));
        match arg.as_str() {
            "--limit" => options.limit = Some(value()?.parse().map_err(|_| Error::Invalid("--limit takes a number".into()))?),
            "--details" => options.details = Some(value()?.into()),
            "--record" => options.record = Some(value()?.into()),
            "--note" => options.note = Some(value()?),
            path if options.parquet.as_os_str().is_empty() && !path.starts_with("--") => options.parquet = path.into(),
            other => return Err(Error::Invalid(format!("unexpected argument: {other}\n{USAGE}"))),
        }
    }
    if options.parquet.as_os_str().is_empty() {
        return Err(Error::Invalid(USAGE.into()));
    }
    Ok(options)
}

fn cord_run(args: &[String]) -> Result<()> {
    let options = parse_options(args)?;
    let sha256 = file_sha256(&options.parquet)?;
    let split = ll_eval::cord::split_name(&options.parquet);
    let workspace = env::temp_dir().join(format!("ll-eval-cord-{}", process::id()));
    let _ = fs::remove_dir_all(&workspace);
    let mut app = ll_app::App::open(&workspace)?;
    let settings = app.settings()?;
    let mut details = options.details.as_ref().map(|path| File::create(path).map(BufWriter::new)).transpose()?;

    let mut card = Scorecard::default();
    let mut latencies = Vec::new();
    let mut seen = 0usize;
    let started = Instant::now();
    let count = read_cord_parquet(&options.parquet, |receipt| {
        if options.limit.is_some_and(|limit| seen >= limit) {
            return Ok(());
        }
        seen += 1;
        let clock = Instant::now();
        let imported = app.import(&format!("{}.img", receipt.id), &receipt.image);
        let elapsed_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let (prediction, text) = match imported {
            Ok(outcome) => {
                let id = ll_core::DocId::from_str(&outcome.document.id).map_err(|e| Error::Invalid(e.to_string()))?;
                let detail = app.detail(id)?;
                (predict(&detail), detail.text)
            }
            // Refused at admission (not a supported image): scored as unreadable.
            Err(Error::Invalid(_)) => (Prediction { routing: Routing::Failed, ..Prediction::default() }, None),
            Err(error) => return Err(error),
        };
        latencies.push(elapsed_ms);
        let score = card.add(&receipt.truth, &prediction);
        eprint!("\r{} receipts read", seen);
        if let Some(out) = details.as_mut() {
            let line = serde_json::json!({
                "id": receipt.id,
                "ms": (elapsed_ms * 10.0).round() / 10.0,
                "truth": { "total": receipt.truth.total, "subtotal": receipt.truth.subtotal, "tax": receipt.truth.tax, "items": receipt.truth.items },
                "predicted": { "total": prediction.total, "subtotal": prediction.subtotal, "tax": prediction.tax, "items": prediction.items },
                "score": score,
                // What OCR read, for error analysis. Details files stay outside the repository.
                "text": text,
            });
            writeln!(out, "{line}")?;
        }
        Ok(())
    })?;
    eprintln!();
    if let Some(mut out) = details {
        out.flush()?;
    }
    drop(app);
    let _ = fs::remove_dir_all(&workspace);

    let run = serde_json::json!({
        "id": format!("{}-cord-v2-{split}-rules-vision", today()),
        "date": today(),
        "note": options.note,
        "dataset": {
            "name": "CORD v2",
            "source": "https://huggingface.co/datasets/naver-clova-ix/cord-v2",
            "split": split,
            "file": options.parquet.file_name().map(|name| name.to_string_lossy().into_owned()),
            "sha256": sha256,
            "licence": "CC BY 4.0",
            "receipts_in_file": count,
            "receipts_scored": card.documents,
        },
        "system": {
            "ledgerlens": env!("CARGO_PKG_VERSION"),
            "pipeline": "ll-app import: PDF text layer or on-device OCR, then the ll-extract rules tier",
            "ocr": if ll_vision::is_available() { "Apple Vision (on-device)" } else { "none" },
            "models": [],
            "default_currency": settings.default_currency,
            "review_threshold": (f64::from(settings.review_threshold) * 100.0).round() / 100.0,
            "scoring": "Amounts compared as numbers in hundredths of the currency unit; currency is not scored (CORD has no currency labels). A wrong value counts as one false positive and one false negative. Line-item prices are matched as a multiset.",
        },
        "hardware": hardware(),
        "results": card.report(),
        "latency_ms_per_receipt": latency_summary(&mut latencies),
        "wall_clock_seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        // Includes the parquet reader, which holds the split's row group (all images) in memory.
        "peak_rss_mb_including_dataset": peak_rss_mb(),
    });
    println!("{}", serde_json::to_string_pretty(&run).map_err(Error::storage)?);
    if let Some(path) = options.record {
        record(&path, run)?;
    }
    Ok(())
}

fn predict(detail: &ll_app::DocumentDetail) -> Prediction {
    let field = |name: FieldName| {
        detail.fields.iter().find(|field| field.name == name).and_then(|field| hundredths(&field.value))
    };
    let routing = match detail.summary.status.as_str() {
        "extracted" | "approved" | "done" => Routing::Ready,
        "failed" => Routing::Failed,
        _ => Routing::NeedsReview,
    };
    Prediction {
        total: field(FieldName::Total),
        subtotal: field(FieldName::Subtotal),
        tax: field(FieldName::Tax),
        items: detail.line_items.iter().map(|item| to_hundredths(item.amount_minor, &item.currency)).collect(),
        routing,
    }
}

fn to_hundredths(minor: i64, currency: &str) -> i64 {
    match ll_core::Currency::from_code(currency).map(|currency| currency.exponent()) {
        Some(0) => minor * 100,
        _ => minor,
    }
}

fn latency_summary(latencies: &mut [f64]) -> serde_json::Value {
    if latencies.is_empty() {
        return serde_json::Value::Null;
    }
    latencies.sort_by(f64::total_cmp);
    let pick = |q: f64| latencies[((latencies.len() - 1) as f64 * q).round() as usize];
    let round = |value: f64| (value * 10.0).round() / 10.0;
    serde_json::json!({
        "mean": round(latencies.iter().sum::<f64>() / latencies.len() as f64),
        "p50": round(pick(0.5)),
        "p95": round(pick(0.95)),
        "max": round(latencies[latencies.len() - 1]),
    })
}

/// Peak resident memory of this process (the whole run, including the parquet reader). Not
/// measured on Windows, which has no `getrusage`; the benchmark is recorded on a Mac.
#[cfg(not(unix))]
fn peak_rss_mb() -> Option<f64> {
    None
}

/// Peak resident memory of this process (the whole run, including the parquet reader).
#[cfg(unix)]
fn peak_rss_mb() -> Option<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes a `rusage` into the pointer it is given and reads nothing else.
    let ok = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0;
    // SAFETY: initialised by the successful call above (and zeroed before it).
    let max = unsafe { usage.assume_init() }.ru_maxrss as f64;
    // macOS reports bytes; Linux reports kilobytes.
    let bytes = if cfg!(target_os = "macos") { max } else { max * 1024.0 };
    ok.then(|| (bytes / (1024.0 * 1024.0) * 10.0).round() / 10.0)
}

fn hardware() -> serde_json::Value {
    let command = |program: &str, args: &[&str]| {
        process::Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let memory_gb = command("sysctl", &["-n", "hw.memsize"])
        .and_then(|bytes| bytes.parse::<f64>().ok())
        .map(|bytes| (bytes / 1024f64.powi(3)).round());
    serde_json::json!({
        "cpu": command("sysctl", &["-n", "machdep.cpu.brand_string"]),
        "cores": std::thread::available_parallelism().map(usize::from).ok(),
        "memory_gb": memory_gb,
        "os": command("sw_vers", &["-productVersion"]).map(|version| format!("macOS {version}")),
        "arch": env::consts::ARCH,
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
    })
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Appends a run to a `results.json` (`{"schema_version": 1, "runs": [...]}`).
fn record(path: &Path, run: serde_json::Value) -> Result<()> {
    let mut results: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path)?).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    let runs = results
        .get_mut("runs")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| Error::Invalid(format!("{} has no runs list", path.display())))?;
    // Several runs on one day share a base id; later ones get a suffix so ids stay unique.
    let mut run = run;
    if let Some(base) = run["id"].as_str().map(str::to_string) {
        let taken = |id: &str| runs.iter().any(|existing| existing["id"] == id);
        let mut id = base.clone();
        let mut n = 2;
        while taken(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        run["id"] = id.into();
    }
    runs.push(run);
    fs::write(path, serde_json::to_string_pretty(&results).map_err(Error::storage)? + "\n")?;
    Ok(())
}

fn today() -> String {
    ll_extract::pipeline::date_from_unix(ll_core::now_unix()).to_string()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_and_latency_summary() {
        let args: Vec<String> = ["data.parquet", "--limit", "5", "--record", "results.json", "--note", "v2"].map(String::from).into();
        let options = parse_options(&args).unwrap();
        assert_eq!((options.parquet, options.limit), (PathBuf::from("data.parquet"), Some(5)));
        assert_eq!(options.record, Some(PathBuf::from("results.json")));
        assert_eq!(options.note.as_deref(), Some("v2"));
        assert!(parse_options(&["--limit".to_string()]).is_err());
        assert!(parse_options(&[]).is_err());

        let summary = latency_summary(&mut [30.0, 10.0, 20.0]);
        assert_eq!((summary["p50"].as_f64(), summary["max"].as_f64(), summary["mean"].as_f64()), (Some(20.0), Some(30.0), Some(20.0)));
        assert_eq!(to_hundredths(1_200, "JPY"), 120_000);
        assert_eq!(to_hundredths(1_250, "USD"), 1_250);
    }

    #[test]
    fn runs_are_appended_to_the_results_file() {
        let path = env::temp_dir().join(format!("ll-eval-results-{}.json", process::id()));
        fs::write(&path, r#"{"schema_version": 1, "runs": []}"#).unwrap();
        record(&path, serde_json::json!({ "id": "a" })).unwrap();
        record(&path, serde_json::json!({ "id": "b" })).unwrap();
        record(&path, serde_json::json!({ "id": "b" })).unwrap();
        let results: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(results["runs"][1]["id"], "b");
        assert_eq!(results["runs"][2]["id"], "b-2", "ids stay unique");
        fs::write(&path, "[]").unwrap();
        assert!(record(&path, serde_json::json!({})).is_err());
        fs::remove_file(&path).unwrap();
    }
}
