import type { Settings, SystemInfo, WatchStatus } from "@ledgerlens/bindings";
import {
  CircleAlert,
  Eye,
  FolderSearch,
  Keyboard,
  Moon,
  Pause,
  Play,
  RefreshCw,
  ScanText,
  ShieldCheck,
  Sun,
} from "lucide-react";
import { useEffect, useState } from "react";
import { currencies } from "../data";
import { timeAgo } from "../format";

export function SettingsView({
  settings,
  info,
  save,
  theme,
  setTheme,
  showShortcuts,
  watch,
  watching,
  scanning,
  chooseWatchFolder,
  setWatching,
  forgetWatch,
  scanNow,
  folderWatching,
}: {
  settings: Settings | null;
  info: SystemInfo | null;
  save: (settings: Settings) => Promise<boolean>;
  theme: "dark" | "light";
  setTheme: (theme: "dark" | "light") => void;
  showShortcuts: () => void;
  watch: WatchStatus | null;
  watching: boolean;
  scanning: boolean;
  chooseWatchFolder: () => void;
  setWatching: (enabled: boolean) => void;
  forgetWatch: () => void;
  scanNow: () => void;
  /** False where no folder can be chosen (phones); the panel then explains instead. */
  folderWatching: boolean;
}) {
  const [draft, setDraft] = useState<Settings | null>(settings);
  const [saving, setSaving] = useState(false);
  useEffect(() => setDraft(settings), [settings]);

  if (!draft || !settings) {
    return <div className="loading-state">Loading settings…</div>;
  }
  const changed = JSON.stringify(draft) !== JSON.stringify(settings);
  const percent = Math.round(draft.reviewThreshold * 100);

  return (
    <section className="page settings-page">
      <div className="settings-main">
      <form
        className="panel settings-form"
        onSubmit={async (event) => {
          event.preventDefault();
          setSaving(true);
          await save(draft);
          setSaving(false);
        }}
      >
        <div className="section-label">
          <span>Workspace</span>
        </div>
        <label className="setting">
          <span>
            <strong>Name</strong>
            <small>Shown in the sidebar and title bar.</small>
          </span>
          <input
            value={draft.workspaceName}
            maxLength={60}
            onChange={(event) => setDraft({ ...draft, workspaceName: event.target.value })}
          />
        </label>
        <label className="setting">
          <span>
            <strong>Default currency</strong>
            <small>Used to read amounts when a document doesn't say which currency it is in.</small>
          </span>
          <select
            value={draft.defaultCurrency}
            onChange={(event) => setDraft({ ...draft, defaultCurrency: event.target.value })}
          >
            {currencies.map((currency) => (
              <option key={currency}>{currency}</option>
            ))}
          </select>
        </label>
        <label className="setting">
          <span>
            <strong>Review threshold · {percent}%</strong>
            <small>
              Fields read with less confidence than this go to review. Higher is stricter. Applies
              to documents read from now on, or read again.
            </small>
          </span>
          <input
            type="range"
            min={50}
            max={99}
            value={percent}
            onChange={(event) =>
              setDraft({ ...draft, reviewThreshold: Number(event.target.value) / 100 })
            }
            aria-label="Review threshold"
          />
        </label>
        <div className="settings-actions">
          <button
            type="button"
            className="button"
            disabled={!changed || saving}
            onClick={() => setDraft(settings)}
          >
            Discard
          </button>
          <button type="submit" className="button button-primary" disabled={!changed || saving}>
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </form>

      {watch && !folderWatching && !watch.folder && (
        <section className="panel watch-panel" aria-labelledby="watch-heading">
          <div className="section-label">
            <span id="watch-heading">Watched folder</span>
            <span className="tag">Mac only</span>
          </div>
          <p className="dim">
            Watching a folder for new documents needs the Ledgerlens desktop app on a Mac. Here,
            add documents with the Add button
            {info?.ocrAvailable
              ? ", including photos taken with the camera."
              : ". PDFs with text are read; enter photos and scans by hand."}
          </p>
        </section>
      )}
      {watch && (folderWatching || watch.folder) && (
        <WatchPanel
          watch={watch}
          watching={watching}
          scanning={scanning}
          chooseWatchFolder={chooseWatchFolder}
          setWatching={setWatching}
          forgetWatch={forgetWatch}
          scanNow={scanNow}
        />
      )}
      </div>

      <div className="settings-side">
        <section className="panel">
          <div className="section-label">
            <span>Appearance</span>
          </div>
          <div className="segmented theme-switch" role="radiogroup" aria-label="Theme">
            {(["dark", "light"] as const).map((option) => (
              <button
                key={option}
                role="radio"
                aria-checked={theme === option}
                className={theme === option ? "selected" : ""}
                onClick={() => setTheme(option)}
              >
                {option === "dark" ? <Moon size={13} /> : <Sun size={13} />}
                {option === "dark" ? "Dark" : "Light"}
              </button>
            ))}
          </div>
          <button className="button shortcuts-link" onClick={showShortcuts}>
            <Keyboard size={14} aria-hidden="true" /> Keyboard shortcuts <kbd>?</kbd>
          </button>
        </section>

        <section className="panel">
          <div className="section-label">
            <span>This device</span>
          </div>
          <dl className="kv">
            <div>
              <dt>
                <ScanText size={13} aria-hidden="true" /> OCR
              </dt>
              <dd>{info?.ocrAvailable ? info.ocrEngine : "Not available — enter scans by hand"}</dd>
            </div>
            <div>
              <dt>Documents</dt>
              <dd className="mono">{info?.documents ?? "—"}</dd>
            </div>
            <div>
              <dt>Version</dt>
              <dd className="mono">{info?.version ?? "—"}</dd>
            </div>
          </dl>
          <p className="modal-label">Data folder</p>
          <pre className="path-box mono">{info?.dataDir ?? "—"}</pre>
          <p className="side-note">
            <ShieldCheck size={13} aria-hidden="true" /> Documents, records, and the search index
            stay in this folder. Ledgerlens makes no network requests. The folder is readable only
            by your user account, but it is not encrypted yet.
          </p>
        </section>
      </div>
    </section>
  );
}

function WatchPanel({
  watch,
  watching,
  scanning,
  chooseWatchFolder,
  setWatching,
  forgetWatch,
  scanNow,
}: {
  watch: WatchStatus;
  watching: boolean;
  scanning: boolean;
  chooseWatchFolder: () => void;
  setWatching: (enabled: boolean) => void;
  forgetWatch: () => void;
  scanNow: () => void;
}) {
  const [confirmForget, setConfirmForget] = useState(false);
  useEffect(() => {
    if (!confirmForget) return;
    const timer = window.setTimeout(() => setConfirmForget(false), 3000);
    return () => window.clearTimeout(timer);
  }, [confirmForget]);
  const { scope, lastScan } = watch;
  const skipped = lastScan
    ? [
        lastScan.skippedLinks && `${lastScan.skippedLinks} link${lastScan.skippedLinks === 1 ? "" : "s"} not followed`,
        lastScan.skippedLarge && `${lastScan.skippedLarge} over ${scope.maxFileMb} MB`,
        lastScan.skippedDeep && `${lastScan.skippedDeep} folder${lastScan.skippedDeep === 1 ? "" : "s"} too deep`,
        lastScan.rejected && `${lastScan.rejected} not a supported document`,
        lastScan.waiting && `${lastScan.waiting} waiting for the next check`,
        lastScan.truncated && `more than ${scope.maxFiles.toLocaleString()} files; the rest wait`,
      ].filter(Boolean)
    : [];

  return (
    <section className="panel watch-panel" aria-labelledby="watch-heading">
      <div className="section-label">
        <span id="watch-heading">Watched folder</span>
        <span className={`tag ${watch.folder ? (watch.enabled ? "ready" : "queued") : ""}`}>
          {watch.folder ? (watch.enabled ? "On" : "Paused") : "Off"}
        </span>
      </div>

      {watch.folder ? (
        <>
          <pre className="path-box mono">{watch.folder}</pre>
          {lastScan?.error && (
            <div className="notice error">
              <CircleAlert size={15} aria-hidden="true" />
              <span>{lastScan.error}</span>
            </div>
          )}
          <dl className="kv watch-stats">
            <div>
              <dt>Imported from this folder</dt>
              <dd className="mono">{watch.imported}</dd>
            </div>
            <div>
              <dt>Last checked</dt>
              <dd>
                {scanning
                  ? "Checking now…"
                  : lastScan && lastScan.at
                    ? `${timeAgo(lastScan.at)} · ${lastScan.found} file${lastScan.found === 1 ? "" : "s"} in scope`
                    : watch.enabled
                      ? "Not yet"
                      : "Paused"}
              </dd>
            </div>
            {skipped.length > 0 && (
              <div>
                <dt>Skipped</dt>
                <dd>{skipped.join(" · ")}</dd>
              </div>
            )}
          </dl>
          <div className="watch-actions">
            <button className="button" onClick={() => setWatching(!watch.enabled)}>
              {watch.enabled ? <Pause size={14} aria-hidden="true" /> : <Play size={14} aria-hidden="true" />}
              {watch.enabled ? "Pause" : "Resume"}
            </button>
            <button className="button" onClick={scanNow} disabled={!watch.enabled || scanning || !watching}>
              <RefreshCw size={14} aria-hidden="true" /> Check now
            </button>
            <button className="button" onClick={chooseWatchFolder}>
              <FolderSearch size={14} aria-hidden="true" /> Change folder…
            </button>
            <span className="actions-spacer" />
            <button
              className={`button ${confirmForget ? "danger" : ""}`}
              onClick={() => (confirmForget ? forgetWatch() : setConfirmForget(true))}
            >
              {confirmForget ? "Click again to stop" : "Stop watching"}
            </button>
          </div>
        </>
      ) : (
        <div className="watch-empty">
          <p>
            Pick a folder where you save invoices and receipts, such as a scanner or downloads
            folder. New documents in it are added to your inbox automatically.
          </p>
          <button className="button button-primary" onClick={chooseWatchFolder}>
            <Eye size={14} aria-hidden="true" /> Watch a folder…
          </button>
        </div>
      )}

      <ul className="watch-scope">
        <li>
          Reads {scope.fileTypes.join(", ")} files up to {scope.maxFileMb} MB, in the folder and up
          to {scope.maxDepth} levels of subfolders, every {scope.intervalSecs} seconds.
        </li>
        <li>Copies files into Ledgerlens. Nothing in the folder is changed, moved, or deleted.</li>
        <li>Skips hidden files and never follows links to other locations.</li>
        <li>
          A document you delete here isn't imported again unless the file changes. Pausing or
          stopping keeps documents already imported.
        </li>
      </ul>
    </section>
  );
}
