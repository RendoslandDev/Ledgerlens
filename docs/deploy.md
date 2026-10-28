# Shipping Ledgerlens

How to put Ledgerlens in people's hands:
- the website on the internet;
- desktop installers as downloads;
- the iPhone app on the App Store;
- the Android app on Google Play.

Ledgerlens is local-first: every copy reads and stores documents on the person's own device, and
the app makes no network requests. "Deploying" therefore means publishing the website and the
installers; there is no server to run and no user data to host.

Steps that need accounts, payment, or signing keys are yours to do; they are marked **(you)**.
Store fees and rules change, so check the linked pages for current terms before you pay.

## 0. Before the first public release

Do these once, before anything goes to a store.

1. **Confirm the app identifier.** It is `com.ledgerlens.app` in
   `apps/desktop/src-tauri/tauri.conf.json`. It was renamed from `com.ledgerlens.desktop` before
   any release, and `gen/apple` and `gen/android` were generated with the new id.
   - The App Store and Google Play tie your listing to this id, and it can't be changed after the
     first upload. Both stores only need it to be unique; if it's already taken there, change it
     now, before the first upload.
   - On desktop, changing it also moves the data folder, so existing local installs would start
     with an empty workspace.
2. **Generate every icon size** from the source icon:
   ```sh
   cd apps/desktop
   npx tauri icon src-tauri/icons/icon.svg
   ```
3. **Publish a privacy policy and a support page (you).** Both stores require a privacy policy
   URL even when no data is collected. The honest policy is short:
   - Ledgerlens collects nothing and sends nothing.
   - Documents stay on the device.
   - The website loads its font from Google Fonts.
4. **Revisit `docs/threat-model.md`.** It lists "shipping installers" as a review trigger.
   - Phones are lost more often than desktops.
   - iOS and Android encrypt app storage when the device is locked, but Ledgerlens's own
     encryption at rest (roadmap Phase 6) is not built yet. Say so in the listing, or finish
     Phase 6 first.
5. **Bump the version** in `tauri.conf.json` and `apps/desktop/package.json` for every release.
   The stores reject a repeated version or build number.

## 1. The website

`apps/web` is plain HTML and CSS with no build step, so any static host works. All three below
are free for a site this size and give HTTPS.

**GitHub Pages (recommended).** The releases live on GitHub too, so this needs no second
account. `.github/workflows/site.yml` publishes `apps/web` on every push to `main` that touches it.

1. Give Ledgerlens its own GitHub repository (you). It currently lives inside your home-folder
   repository. The site and README link to `RendoslandDev/ledgerlens`; if you pick another name,
   change those links (three in `apps/web/index.html`, two in `README.md`).
2. Settings › Pages › Source: **GitHub Actions** (you).
3. Push to `main`, or run **Actions › Publish website › Run workflow**. The site appears at
   `https://rendoslanddev.github.io/Ledgerlens/`. The path matches the repository name exactly,
   including case: `/ledgerlens/` is a 404.
4. Optional: add a custom domain under Settings › Pages (you). Only enter a domain you own and
   have pointed at GitHub; GitHub redirects every visitor to whatever domain is entered there.

**Cloudflare Pages or Netlify** also work: no build command, output directory `apps/web`.

**After an App Store or Google Play listing goes live,** replace that platform's "Not published yet"
tag in the site's Download section (`#download` in `apps/web/index.html`) with its store link.

## 2. Desktop downloads (GitHub Releases)

`.github/workflows/release.yml` builds macOS installers (Apple silicon and Intel) with
`tauri-apps/tauri-action` when you push a tag such as `v0.1.0`, and attaches them to a draft GitHub
Release for you to publish. The site's and README's download links point at the latest release.

**What each platform supports:**
- **macOS 13 or later:** everything, including OCR and folder watching. The Apple silicon build was
  built and launched locally; the Intel build has not been launched yet.
- **Windows and Linux:** not built or tested, so they are left out of the release matrix
  (commented out in `release.yml`). Expect text PDFs only there: no OCR and no folder watching.
  Turn them on after a CI build produces installers that open and import a PDF.

**Without Apple signing (the default).** The app is ad-hoc signed (`signingIdentity: "-"` in
`tauri.conf.json`, and the workflow falls back to `-` when the secret is empty). It runs, but
Gatekeeper blocks the first launch until the person chooses **Open Anyway** in System Settings ›
Privacy & Security. The README walks testers through it. Fine for a test group; not for a public
launch.

**macOS signing and notarization (you).** With these, the app opens without any warning.

1. Join the Apple Developer Program: <https://developer.apple.com/programs/>. It costs $99 a year
   at the time of writing.
2. Create a **Developer ID Application** certificate. Export it from Keychain Access as a `.p12`
   file with a password.
3. Create an app-specific password for your Apple ID at <https://account.apple.com>.
4. Add these repository secrets in GitHub (Settings → Secrets and variables → Actions):

   | Secret | Value |
   | --- | --- |
   | `APPLE_CERTIFICATE` | `base64 -i certificate.p12` |
   | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
   | `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
   | `APPLE_ID` | your Apple ID email |
   | `APPLE_PASSWORD` | the app-specific password |
   | `APPLE_TEAM_ID` | your 10-character team id |

**Windows signing (you).** Unsigned installers show a SmartScreen warning. Use an OV/EV
code-signing certificate or Azure Trusted Signing, following
<https://v2.tauri.app/distribute/sign/windows/>.

**Release:**
```sh
git tag v0.1.0 && git push origin v0.1.0
```
Then review the draft release on GitHub and publish it.

## 3. iPhone and iPad (App Store)

The iOS app shares the desktop app's Rust core and interface. It reads PDFs and photos with
Apple Vision on the device. Phones have no folder picker, so folder import and folder watching
are hidden there; documents come in through the Add button, which offers the camera, Photos, and
Files.

**One-time setup on the Mac that builds:**
1. Xcode, from the App Store.
2. The Rust iOS targets:
   ```sh
   rustup target add aarch64-apple-ios aarch64-apple-ios-sim
   ```
3. XcodeGen and CocoaPods: `brew install xcodegen cocoapods`.
4. `npx tauri ios init` in `apps/desktop`, which creates `src-tauri/gen/apple`. This step is
   already done in this repository.

**Try it in the simulator:**
```sh
cd apps/desktop
npx tauri ios dev                                  # live reload against the Vite dev server
npx tauri ios build --target aarch64-sim --debug   # or a standalone simulator build
```

**Publish (you):**
1. Join the Apple Developer Program (the same membership as for macOS signing).
2. In the developer portal, register an App ID with your final bundle identifier.
3. Set your team in `tauri.conf.json` so Xcode can sign:
   ```json
   "bundle": { "iOS": { "developmentTeam": "YOURTEAMID" } }
   ```
4. Create the app record in App Store Connect (<https://appstoreconnect.apple.com>). Fill in:
   - name, subtitle, and description;
   - keywords and category (Finance or Business);
   - privacy policy and support URLs;
   - age rating.
5. Answer **App Privacy** with "Data Not Collected". Export compliance is answered in the build:
   `ITSAppUsesNonExemptEncryption` is `false` in `src-tauri/Info.ios.plist`.
6. Build the upload:
   ```sh
   npx tauri ios build --export-method app-store-connect
   ```
   The `.ipa` lands under `src-tauri/gen/apple/build`.
7. Upload it with Apple's **Transporter** app (Mac App Store), or from Xcode → Window →
   Organizer.
8. Test through **TestFlight** first: add yourself and a few testers.
9. Add screenshots. App Store Connect lists the required device sizes, currently the 6.9-inch
   iPhone, plus iPad if you keep iPad support. Then submit for review. In the review notes, say
   there is no account or login, and that everything runs on the device.

The camera and Photos permission messages are in `src-tauri/Info.ios.plist`, and Apple reviews
their wording. Exported CSV files appear in the Files app under On My iPhone → Ledgerlens.

## 4. Android (Google Play)

Android builds from the same code. For the first release, Android reads text PDFs. It has no
built-in on-device OCR like Apple Vision, so photos and scans go to manual entry with a note
saying so. Adding OCR (for example ML Kit) is a separate step for later.

**One-time setup (about 6–8 GB).** This Mac uses the command-line route; Android Studio works
too and installs the same pieces.

1. Install a JDK and Google's Android command-line tools:
   ```sh
   brew install openjdk@17
   brew install --cask android-commandlinetools
   ```
2. Point your shell at them. Add this to `~/.zshrc`:
   ```sh
   export JAVA_HOME="/opt/homebrew/opt/openjdk@17"
   export ANDROID_HOME="$HOME/Library/Android/sdk"
   export NDK_HOME="$ANDROID_HOME/ndk/29.0.14206865"
   export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"
   ```
3. Accept the SDK licence (you), then install the platform, build tools, NDK, emulator, and an
   emulator image:
   ```sh
   sdkmanager --sdk_root="$ANDROID_HOME" --licenses
   sdkmanager --sdk_root="$ANDROID_HOME" platform-tools "platforms;android-36" "build-tools;36.1.0" \
     "ndk;29.0.14206865" emulator "system-images;android-36;google_apis;arm64-v8a"
   ```
4. Add the Rust Android targets:
   ```sh
   rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
   ```
5. Generate the Android project, which creates `src-tauri/gen/android`. This step is already done
   in this repository.
   ```sh
   cd apps/desktop && npx tauri android init
   ```
6. Create an emulator once and start it:
   ```sh
   avdmanager create avd -n ledgerlens -k "system-images;android-36;google_apis;arm64-v8a"
   emulator -avd ledgerlens &
   ```
7. Run the app on it with live reload: `npx tauri android dev`.

**Signing key (you).** Keep the keystore and its passwords safe and backed up. Losing them blocks
updates unless you use Play App Signing.
```sh
keytool -genkey -v -keystore ~/ledgerlens-upload.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload
```
Create `src-tauri/gen/android/keystore.properties` (never commit it):
```properties
storeFile=/Users/you/ledgerlens-upload.jks
storePassword=…
keyAlias=upload
keyPassword=…
```
Then load it in `src-tauri/gen/android/app/build.gradle.kts` as a `release` signing config,
following <https://v2.tauri.app/distribute/sign/android/>.

**Build the upload:**
```sh
npx tauri android build --aab
```
The `.aab` lands under `src-tauri/gen/android/app/build/outputs/bundle/`.

**Publish (you):**
1. Create a Google Play Console account (<https://play.google.com/console>). There is a one-time
   fee, $25 at the time of writing, plus identity verification.
2. Create the app. Fill in:
   - store listing, with screenshots and a 512×512 icon (from `npx tauri icon`);
   - privacy policy URL;
   - content rating;
   - target audience.
3. Answer **Data safety**: no data collected, no data shared.
4. Enroll in **Play App Signing**, then upload the `.aab` to the **internal testing** track.
5. Promote to closed testing, then production. Newer personal developer accounts must run a
   closed test with a minimum number of testers for a minimum period before production access.
   Check the current requirement in Play Console.

## 5. Each release, in order

1. Bump the version.
2. Check that everything passes:
   - `cargo test --workspace`;
   - `npm --prefix apps/desktop run check`;
   - `cargo clippy --workspace --all-targets -- -D warnings`.
3. Re-run the benchmark if the extraction rules changed (`benchmarks/README.md`).
4. Tag and push to build the desktop release; publish the draft release.
5. Build and upload the iOS `.ipa` and Android `.aab`; release through TestFlight and internal
   testing, then to production.
6. Update the website's Download section if links or versions changed.
