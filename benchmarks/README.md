# Benchmarks

Measured runs live in `results.json`, one entry per run, each with the dataset file and its
SHA-256, the pipeline and settings, the hardware, and the scores. Nothing is recorded that was not
measured. Datasets stay outside the repository.

## CORD v2 (test split)

- **Source:** `naver-clova-ix/cord-v2` on Hugging Face,
  `data/test-00000-of-00001-9c204eb3f4e11791.parquet`, 234,202,795 bytes, SHA-256
  `51c65f1788faff392abe2a0b55b023eb23e9be551c509138eaa3a832514224e7`.
- **Licence:** CC BY 4.0. CORD is by Park et al., "CORD: A Consolidated Receipt Dataset for
  Post-OCR Parsing" (NeurIPS 2019 Document Intelligence Workshop).
- **Contents:** 100 Indonesian restaurant and shop receipts, photographed.

The **validation** split (`data/validation-00000-of-00001-cc3c5779fe22e8ca.parquet`, 242,080,800
bytes, SHA-256 `0d0f6dac11fdcc549de2746aa9f53136a3bc22a2a1aff2b0b847f7622ad60c15`, 100 receipts) is
used to develop the rules. The test split is only scored, so its numbers say how the rules do on
receipts they were not tuned on.

Download it, check it, then run:

```sh
mkdir -p ~/.cache/ledgerlens/datasets/cord-v2 && cd ~/.cache/ledgerlens/datasets/cord-v2
curl -fLo test-00000-of-00001.parquet \
  https://huggingface.co/datasets/naver-clova-ix/cord-v2/resolve/main/data/test-00000-of-00001-9c204eb3f4e11791.parquet
echo "51c65f1788faff392abe2a0b55b023eb23e9be551c509138eaa3a832514224e7  test-00000-of-00001.parquet" | shasum -a 256 -c -
cd -
cargo run --release -p ll-eval -- cord-run ~/.cache/ledgerlens/datasets/cord-v2/test-00000-of-00001.parquet \
  --details /tmp/cord-details.jsonl --record benchmarks/results.json
# --details holds scores and OCR text per receipt; keep it outside the repository.
```

### How a run works

- **What is measured:** the same path the desktop app uses. Each receipt is imported into a
  temporary workspace through `ll_app::App::import`: the type is checked from the file's bytes,
  the image is read with Apple Vision OCR on the device, and then the `ll-extract` rules run. The
  stored record is then compared with CORD's labels.
- **Settings:** a new workspace's defaults (USD default currency, 85% review threshold).
- **Fields scored:** total, subtotal, tax, and line-item prices.
  - A wrong value counts as one false positive and one false negative.
  - Line prices are matched as a multiset.
- **Amounts:** compared as numbers. "60.000" and "60,000" are both sixty thousand.
- **Currency:** not scored. CORD has no currency labels.
- **Routing:** each run also reports how many receipts skipped review, and how many of those
  were fully correct (`ready_precision`).
- **Repeatability:** the scores are deterministic; two runs on the same machine gave identical
  results.

### First run: 2026-10-27, rules plus Apple Vision, Apple M2, 8 GB, macOS 26.2

| Field | Precision | Recall | F1 |
| --- | --- | --- | --- |
| Total | 0.80 | 0.69 | 0.74 |
| Subtotal | 0.96 | 0.66 | 0.78 |
| Tax | 1.00 | 0.29 | 0.45 |
| Line-item prices | 0.50 | 0.00 | 0.01 |

- **All labelled fields correct:** 42 of 100 receipts.
- **Review:** all 100 went to review, so nothing wrong was accepted without review.
  - Most CORD receipts lack the vendor, date, and number that Ledgerlens requires before skipping
    review.
  - OCR'd receipts also skip review only when subtotal plus tax equals the total.
- **Speed:** median 147 ms per receipt, p95 345 ms, slowest 689 ms. The whole split took 18 s.
- **Memory:** peak 685 MB, which includes the parquet reader holding all 100 images.
- Timing varies between runs; an earlier, unrecorded run of the same code had a 205 ms median.

Where the rules miss, from the per-receipt details:

- **Totals:**
  - 14 were wrong.
    - At least four came from item-count lines read as the total: "Total quantity 3",
      "TOTAL QTY 2" (twice), and "Total Rty 21", which is OCR's reading of "Qty".
    - One is OCR reading "281,435" as "281-435", giving −435.
    - One receipt prints "9,500,00", which was read as 950,000.
    - One matches the total before 10% tax.
  - 15 were not found at all.
- **Tax and subtotal:** mostly missed rather than wrong: 29 taxes and 21 subtotals were not found,
  against 0 and 1 wrong. Finding which labels the rules fail on needs a look at the OCR text.
- **Line items:** read from only 2 of 100 receipts. The rules read items only below a table
  header (such as "Description … Amount"), and these receipts print items with no header, for
  example "Y.B. BAT 46000".

### Second run: rules v2, same machine (`…-test-rules-vision-2`)

The rules were changed using the **validation** split only, then the test split was scored once.

- **IDR:** IDR is supported, and "Rp" before an amount marks rupiah.
- **Split numbers:** thousands split by OCR are rejoined ("16, 500", "121 798").
- **Totals:**
  - Item-count lines ("TOTAL QTY 2", "Total Rty 21") are never totals.
  - Cells naming a payment ("Change 8,000") are ignored.
- **More labels:**
  - tax: PB1, Pajak (Resto), P.Rest, PPN;
  - subtotal: SUBTTL, Net Amt, Net Sales.
- **Line items:** without a table header, rows above the first summary row that read as purchases
  are items. Header rows (table, cashier, phone, date) and payment rows are excluded.

| Field | F1 before | F1 after | Precision after | Recall after |
| --- | --- | --- | --- | --- |
| Total | 0.74 | 0.77 | 0.87 | 0.69 |
| Subtotal | 0.78 | 0.84 | 0.96 | 0.75 |
| Tax | 0.45 | 0.79 | 0.93 | 0.68 |
| Line-item prices | 0.01 | 0.68 | 0.87 | 0.56 |

- **All labelled fields correct:** 57 of 100 receipts, up from 42.
- **Review:** all 100 still went to review, so nothing was accepted without a person checking it.
- **Fixed against regressed:**
  - Subtotal: 6 more receipts right, none worse.
  - Tax: 16 more right, none worse.
  - Total: 2 more right, 2 worse.
- **On validation** (the development data): total F1 0.86 → 0.90, subtotal 0.79 → 0.89, tax
  0.45 → 0.89, line prices 0.00 → 0.74.

**The two total regressions, and a fix after the run.** Two test receipts print the total with a
count in brackets: "TOTAL (2 item) 120,000" and "TOTAL (Qty=4.00)240.000". The new count-line rule
dropped them.

The rule was fixed afterwards: a bracketed count no longer disqualifies a total line. The fix is
unit-tested and leaves validation unchanged. But it was prompted by test receipts, so the test
split is no longer an untouched held-out set for it. The recorded second run is from before the
fix, and no test run was recorded after it.

**Still open:**
- **Totals:** some are missed where OCR reads the total line poorly, and some are printed in
  thousands ("TOTAL 20" for 20,000).
- **Review:** CORD receipts still never skip review, because they lack the vendor, date, and
  number that Ledgerlens requires before it does.

## SROIE

SROIE is not used: its licence is research-only, and unclear for this project. `ll-eval sroie`
still inventories a local copy.
