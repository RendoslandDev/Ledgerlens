# Ledgerlens

Ledgerlens reads invoices and receipts on your own Mac. It pulls out the vendor, dates, totals,
tax, and line items, sends anything it isn't sure about to review, and turns the rest into
records you can search, question, and export to CSV for QuickBooks or Xero.

Nothing is uploaded and there is no account. Ledgerlens makes no network requests.

**Status: early preview for macOS.** Expect rough edges, and please report them.

## Install on a Mac

Requires macOS 13 (Ventura) or later.

1. From the [latest release](https://github.com/RendoslandDev/ledgerlens/releases/latest), download
   the `.dmg` for your Mac: `aarch64` for Apple silicon (M1 and later), `x64` for Intel.
2. Open the `.dmg` and drag **Ledgerlens** into **Applications**.
3. Open Ledgerlens. The preview isn't notarized by Apple yet, so macOS stops the first launch with
   a warning. Close the warning.
4. Open **System Settings › Privacy & Security**, scroll to Security, and choose **Open Anyway**
   next to the Ledgerlens message. Confirm with your password or Touch ID.

After that, Ledgerlens opens normally.

## What to try

- **Add documents:** PDFs, PNG, JPG, and TIFF, up to 25 MB each. Digital PDFs are read from their
  text; scans and photos are read with Apple's on-device Vision OCR.
- **Review:** work through the inbox as a table (J/K to move, A to approve) or drag cards on the
  board. Anything uncertain lands in **Needs review**.
- **Ask:** questions about reviewed spend are answered by a SQL query that is shown to you.
- **Export:** CSV in a canonical, QuickBooks, or Xero shape.
- **Watch a folder** (optional, in Settings): new files in one folder you choose are copied in.

## Your data

Everything lives in `~/Library/Application Support/com.ledgerlens.app`, readable only by your
user account. It is not encrypted at rest yet. To remove everything, quit Ledgerlens and delete
that folder and the app.

## Report a problem

Open an [issue](https://github.com/RendoslandDev/ledgerlens/issues) with what you did, what you
expected, and your macOS version. Please don't attach real invoices; describe the layout or share
a redacted copy instead.

## Other platforms

- **iPhone and iPad:** built and tested in the simulator; not on the App Store yet.
- **Android:** built and tested on an emulator; reads text PDFs, with photos and scans entered by
  hand. Not on Google Play yet.
- **Windows and Linux:** not built or tested yet.

## Build from source

Needs Rust (stable) and Node 22.

```sh
npm --prefix apps/desktop install
npm --prefix apps/desktop run tauri dev
```

Architecture, the threat model, benchmarks, and release steps are in [`docs/`](docs/) and
[`benchmarks/`](benchmarks/).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
