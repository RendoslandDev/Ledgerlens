# Ledgerlens

Ledgerlens reads invoices and receipts on your own device, with bank statements and contracts
coming in Ledgerlens Pro.

- **Invoices and receipts:** the vendor, dates, totals, tax, and line items.
- **Bank statements** (Pro, coming soon): every transaction, checked against the opening and closing
  balances.
- **Contracts** (Pro, coming soon): the parties, start and end dates, notice period, and whether it
  renews, with the last day to give notice.

Anything it isn't sure about goes to review. The rest becomes records you can search and
question. Export invoices and receipts as CSV for QuickBooks or Xero, and (with Pro)
statements as OFX, which QuickBooks Online and Xero import as bank transactions.

Nothing is uploaded and there is no account. Ledgerlens makes no network requests.

**Status: early preview** for macOS, Windows, Linux, and Android. Expect rough edges, and please
report them. The Mac app has had the most use. The Windows and Linux builds are tested
automatically and the Android app on an emulator (see [What has been tested](#what-has-been-tested)),
but they have seen little hands-on use yet.

## Install on a Mac

Requires macOS 13 (Ventura) or later.

1. From the [latest release](https://github.com/RendoslandDev/Ledgerlens/releases/latest), download
   the `.dmg` for your Mac: `aarch64` for Apple silicon (M1 and later), `x64` for Intel.
2. Open the `.dmg` and drag **Ledgerlens** into **Applications**.
3. Open Ledgerlens. The preview isn't notarized by Apple yet, so macOS stops the first launch with
   a warning. Close the warning.
4. Open **System Settings › Privacy & Security**, scroll to Security, and choose **Open Anyway**
   next to the Ledgerlens message. Confirm with your password or Touch ID.

After that, Ledgerlens opens normally.

## Install on Windows

Requires 64-bit Windows 10 or 11.

1. From the [latest release](https://github.com/RendoslandDev/Ledgerlens/releases/latest), download
   `Ledgerlens_<version>_x64-setup.exe`. It installs for your account only and doesn't need an
   administrator. (The `.msi` is there for managed installs.)
2. Run it. The preview isn't code-signed yet, so Windows SmartScreen may say "Windows protected
   your PC". Choose **More info**, then **Run anyway**.
3. Open Ledgerlens from the Start menu.

## Install on Linux

Built on Ubuntu 22.04; needs a 64-bit distribution of that age or newer (glibc 2.35+, WebKitGTK 4.1).

- **Debian or Ubuntu:** download the `.deb`, then `sudo apt install ./Ledgerlens_<version>_amd64.deb`.
- **Fedora and other RPM systems:** download the `.rpm`, then `sudo dnf install ./Ledgerlens-<version>-1.x86_64.rpm`.
- **Anything else:** download the `.AppImage`, make it executable (`chmod +x`), and run it.

## Install on Android

Requires Android 7 or later on a phone or tablet with an ARM processor (nearly all of them).

1. On the phone, open the [latest release](https://github.com/RendoslandDev/Ledgerlens/releases/latest)
   and download `Ledgerlens_<version>_android.apk`.
2. Open the download. Android asks whether to allow installing apps from your browser or Files;
   allow it for that app, then tap **Install**. You can turn the permission off again afterwards.
3. Google Play Protect may say it doesn't recognise the app, because it isn't on Google Play yet;
   choose **Install anyway**.

Updates are installed the same way, over the existing app, and keep your library.

## What to try

- **Add documents:** PDFs, PNG, JPG, and TIFF, up to 25 MB each. Digital PDFs are read from their
  text. On a Mac, scans and photos are read with Apple's on-device Vision OCR; on Windows, Linux, and
  Android there is no OCR yet, so you enter a photo's details by hand.
- **Review:** work through the inbox as a table (J/K to move, A to approve) or drag cards on the
  board. Anything uncertain lands in **Needs review**.
- **Ask:** questions about reviewed spend are answered by a SQL query that is shown to you.
- **Export:** CSV in a canonical, QuickBooks, or Xero shape. Nothing is sent to QuickBooks or
  Xero for you: you import the file.
- **Watch a folder** (optional, in Settings; Mac only for now): new files in one folder you choose
  are copied in.

## Ledgerlens Pro (coming soon)

A paid version, after the App Store launch, adds:

- **Bank statements:** every transaction, with each row's direction (in or out) worked out from
  the running balance. If the opening balance plus the transactions doesn't equal the closing
  balance, the statement goes to review instead of being trusted. Only the last four digits of
  the account number are kept.
- **Contracts:** the key terms (parties, start and end dates, notice period, auto-renewal,
  value) and the last day to give notice; not a clause-by-clause review. An end date worked
  out from the term, and parties and amounts read from prose, are flagged for you to confirm.
- **Statement export as OFX**, one file per account, for QuickBooks Online (Banking › Upload
  from file) or Xero (Import a statement). Re-importing a statement is recognised as a
  duplicate.

These are built and tested on sample documents but switched off in the free download. To try
them from source, run `npx tauri dev --features pro` in `apps/desktop`.

## Your data

Everything lives in one folder on your device:

- **Mac:** `~/Library/Application Support/com.ledgerlens.app`
- **Windows:** `%LOCALAPPDATA%\com.ledgerlens.app` (local, so it never roams to a domain server)
- **Linux:** `~/.local/share/com.ledgerlens.app`
- **Android:** the app's private storage, which other apps can't read

On a Mac and on Linux the folder is readable only by your user account; on Windows it has the
usual protection of your account's local app data. It is not encrypted at rest yet. To remove
everything, quit Ledgerlens, uninstall it, and delete that folder. On Android, uninstalling
removes everything.

## Report a problem

Open an [issue](https://github.com/RendoslandDev/Ledgerlens/issues) with what you did, what you
expected, and your operating system and version. Please don't attach real invoices; describe the layout or share
a redacted copy instead.

## Other platforms

- **iPhone and iPad:** built and tested in the simulator; not on the App Store yet.
- **Android on Google Play:** not yet; for now, install the APK as described above.

## What has been tested

Every change is built and tested on macOS, Windows, and Linux in
[CI](.github/workflows/ci.yml). Beyond the test suites:

- **Mac:** the release `.dmg` was installed, launched, and used by hand.
- **Windows:** CI installs the `-setup.exe` silently and checks that the app starts, creates its
  library, and starts its web view. It then rebuilds the same code with a debugging port open,
  imports a text PDF through the app's interface, and checks what was read. This runs on GitHub's
  Windows Server machines; the `.msi` and everyday Windows 10 or 11 PCs haven't been tried yet.
- **Linux:** CI installs the `.deb` on Ubuntu 22.04 and checks that the app starts and creates its
  library. Importing in the Linux app, the `.AppImage`, and the `.rpm` haven't been tried yet.
- **Android:** the signed release APK was installed on an Android 16 emulator (arm64), and a text
  PDF was imported with the Add button and read correctly. It hasn't been tried on a real phone
  or on 32-bit ARM yet.

## Build from source

Needs Rust 1.96 and Node 22. On Linux, also install WebKitGTK and friends:
`sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`.

```sh
npm --prefix apps/desktop install
npm --prefix apps/desktop run tauri dev
```

Architecture, the threat model, benchmarks, and release steps are in [`docs/`](docs/) and
[`benchmarks/`](benchmarks/).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
