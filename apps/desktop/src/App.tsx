import type {
  Activity,
  DocumentDetail,
  DocumentSummary,
  ExportFormat,
  ExportPlan,
  FieldEdit,
  Report,
  Settings,
  SystemInfo,
  VendorSummary,
  WatchStatus,
} from "@ledgerlens/bindings";
import {
  ArrowDownToLine,
  CheckCircle2,
  CheckCheck,
  CircleAlert,
  Command as CommandIcon,
  Eye,
  FolderInput,
  Keyboard,
  ListFilter,
  Moon,
  Plus,
  Search,
  Settings2,
  ShieldCheck,
  Upload,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type FormEvent,
} from "react";
import { commands, errorMessage, hasBackend, importFile, unwrap } from "./api";
import { folderName } from "./format";
import { CommandPalette, type Command } from "./components/CommandPalette";
import { ExportDialog } from "./components/ExportDialog";
import { ShortcutsSheet } from "./components/ShortcutsSheet";
import { LogoMark } from "./components/Logo";
import { Rail, navItems, type View } from "./components/Rail";
import { displayName, queueState } from "./data";
import { AskView, type AskEntry } from "./views/AskView";
import { InboxView, type InboxLayout, type QueueFilter, type SortKey } from "./views/InboxView";
import { Welcome } from "./components/Welcome";
import { ReportsView } from "./views/ReportsView";
import { SettingsView } from "./views/SettingsView";
import { VendorsView } from "./views/VendorsView";

type Theme = "dark" | "light";
type Toast = { message: string; tone: "ok" | "error" };

const titles: Record<View, string> = {
  inbox: "Inbox",
  ask: "Ask",
  reports: "Reports",
  vendors: "Vendors",
  settings: "Settings",
};

const subtitles: Record<View, string> = {
  inbox: "Review what Ledgerlens read from your paperwork",
  ask: "Questions answered from your reviewed records",
  reports: "Spend across your reviewed records",
  vendors: "Everyone you have paperwork from",
  settings: "Workspace, folder watching, and this device",
};

/** Per-viewer conveniences; the app works the same without them. */
function readPreference(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writePreference(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Applies for this session only.
  }
}

function sortDocuments(documents: DocumentSummary[], sort: SortKey): DocumentSummary[] {
  const sorted = [...documents];
  const name = (d: DocumentSummary) => (d.vendor ?? d.fileName).toLowerCase();
  switch (sort) {
    case "oldest":
      return sorted.reverse();
    case "amount":
      return sorted.sort((a, b) => (b.totalMinor ?? -1) - (a.totalMinor ?? -1));
    case "vendor":
      return sorted.sort((a, b) => name(a).localeCompare(name(b)));
    case "confidence":
      return sorted.sort((a, b) => a.confidence - b.confidence);
    default:
      return sorted;
  }
}
const SUPPORTED = /\.(pdf|png|jpe?g|tiff?)$/i;
const MAX_BYTES = 25 * 1024 * 1024;

function storedTheme(): Theme {
  try {
    return localStorage.getItem("ledgerlens-theme") === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
}

function isTyping(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName))
  );
}

function App() {
  const [theme, setTheme] = useState<Theme>(storedTheme);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("ledgerlens-theme", theme);
    } catch {
      // Theme still applies for this session.
    }
  }, [theme]);

  if (!hasBackend) return <BackendMissing />;
  return <Workspace theme={theme} setTheme={setTheme} />;
}

function BackendMissing() {
  return (
    <main className="backend-missing">
      <LogoMark size={32} />
      <h1>Ledgerlens runs as a desktop app</h1>
      <p>
        Documents are read and stored by the local backend, which this browser tab can't reach.
        Start the desktop app from the repository root:
      </p>
      <pre className="mono">npm --prefix apps/desktop run tauri dev</pre>
    </main>
  );
}

function Workspace({ theme, setTheme }: { theme: Theme; setTheme: (theme: Theme) => void }) {
  const [view, setView] = useState<View>("inbox");
  const [documents, setDocuments] = useState<DocumentSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [detail, setDetail] = useState<DocumentDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [categories, setCategories] = useState<string[]>([]);
  const [report, setReport] = useState<Report | null>(null);
  const [queueFilter, setQueueFilter] = useState<QueueFilter>("all");
  const [search, setSearch] = useState("");
  const [question, setQuestion] = useState("");
  const [entries, setEntries] = useState<AskEntry[]>([]);
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const [dragging, setDragging] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [vendors, setVendors] = useState<VendorSummary[] | null>(null);
  const [sort, setSort] = useState<SortKey>("newest");
  const [inboxLayout, setInboxLayoutState] = useState<InboxLayout>(() =>
    readPreference("ledgerlens-inbox-layout") === "board" ? "board" : "table",
  );
  const [welcomeSkipped, setWelcomeSkipped] = useState(
    () => readPreference("ledgerlens-welcome-skipped") === "1",
  );
  function setInboxLayout(layout: InboxLayout) {
    setInboxLayoutState(layout);
    writePreference("ledgerlens-inbox-layout", layout);
  }
  function skipWelcome() {
    setWelcomeSkipped(true);
    writePreference("ledgerlens-welcome-skipped", "1");
  }
  const [snippets, setSnippets] = useState<Map<string, string>>(new Map());
  const [exportState, setExportState] = useState<{ format: ExportFormat; plan: ExportPlan } | null>(null);
  const [activity, setActivity] = useState<Activity | null>(null);
  const [watch, setWatch] = useState<WatchStatus | null>(null);
  const seenRevision = useRef<number | null>(null);
  const [exporting, setExporting] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const vendorRef = useRef<HTMLInputElement>(null);
  const toastTimer = useRef<number>(undefined);

  // Phones have no folder picker and can't hand files to another app; folder watching needs the
  // macOS folder dialog. Until system info arrives, assume a desktop.
  const isMobile = info?.platform === "ios" || info?.platform === "android";
  const canImportFolder = !isMobile;
  const canWatch = info?.folderWatching ?? !isMobile;
  const canOpenOriginal = !isMobile;

  const reviewCount = documents.filter((document) => document.status === "needs_review").length;
  const filteredDocuments = sortDocuments(
    documents.filter((document) => {
      // On the board, status is the column, so the status filter doesn't apply.
      const matchesStatus =
        inboxLayout === "board" || queueFilter === "all" || queueState(document.status) === queueFilter;
      const needle = search.trim().toLowerCase();
      const matchesSearch =
        !needle ||
        snippets.has(document.id) ||
        [document.vendor, document.invoiceNumber, document.category, document.fileName].some(
          (value) => value?.toLowerCase().includes(needle),
        );
      return matchesStatus && matchesSearch;
    }),
    sort,
  );
  const workspaceName = settings?.workspaceName ?? "Workspace";

  const announce = useCallback((message: string, tone: Toast["tone"] = "ok") => {
    setToast({ message, tone });
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), tone === "error" ? 6000 : 3200);
  }, []);

  const refreshReport = useCallback(() => {
    unwrap(commands.report())
      .then(setReport)
      .catch((error) => announce(errorMessage(error), "error"));
  }, [announce]);

  const refreshList = useCallback(async () => {
    try {
      const list = await unwrap(commands.listDocuments());
      setDocuments(list);
      setLoaded(true);
      return list;
    } catch (error) {
      announce(errorMessage(error), "error");
      setLoaded(true);
      return [];
    }
  }, [announce]);

  useEffect(() => {
    void refreshList().then((list) => setActiveId((current) => current ?? list[0]?.id ?? null));
    commands.categories().then(setCategories).catch(() => setCategories([]));
    unwrap(commands.getSettings())
      .then(setSettings)
      .catch((error) => announce(errorMessage(error), "error"));
    unwrap(commands.systemInfo())
      .then(setInfo)
      .catch(() => undefined);
  }, [refreshList, announce]);

  useEffect(() => {
    document.title = `${workspaceName} · Ledgerlens`;
  }, [workspaceName]);

  // The background worker reads imports and scans the watched folder. Its activity snapshot is
  // cheap to read, so poll it: quickly while documents are being read, slowly otherwise, and
  // reload the list whenever the worker changed something.
  const busyWorker = Boolean(activity && (activity.queued > 0 || activity.reading || activity.scanning));
  useEffect(() => {
    let cancelled = false;
    const poll = () =>
      commands
        .activity()
        .then((next) => {
          if (cancelled) return;
          setActivity(next);
          if (seenRevision.current !== null && seenRevision.current !== next.revision) {
            void refreshList();
            if (view !== "inbox") refreshReport();
          }
          seenRevision.current = next.revision;
        })
        .catch(() => undefined);
    poll();
    const timer = window.setInterval(poll, busyWorker ? 1000 : 4000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [busyWorker, refreshList, refreshReport, view]);

  // Keep the open document current while the worker reads it.
  const activeSummary = documents.find((document) => document.id === activeId);
  const activeStatus = activeSummary?.status;
  useEffect(() => {
    if (!activeId || !detail || detail.summary.id !== activeId) return;
    if (activeStatus && activeStatus !== detail.summary.status) {
      unwrap(commands.getDocument(activeId))
        .then(setDetail)
        .catch(() => undefined);
    }
  }, [activeId, activeStatus, detail]);

  const refreshWatch = useCallback(() => {
    unwrap(commands.watchStatus())
      .then(setWatch)
      .catch((error) => announce(errorMessage(error), "error"));
  }, [announce]);

  useEffect(() => {
    refreshWatch();
  }, [refreshWatch, activity?.revision, activity?.watching]);

  useEffect(() => {
    if (view === "vendors") {
      unwrap(commands.vendors())
        .then(setVendors)
        .catch((error) => announce(errorMessage(error), "error"));
    }
    if (view === "settings") {
      unwrap(commands.systemInfo())
        .then(setInfo)
        .catch((error) => announce(errorMessage(error), "error"));
    }
  }, [view, announce, documents]);

  // Full-text search over document text, debounced; metadata matches stay instant.
  useEffect(() => {
    const query = search.trim();
    if (query.length < 2) {
      setSnippets(new Map());
      return;
    }
    const timer = window.setTimeout(() => {
      unwrap(commands.search(query))
        .then((hits) => setSnippets(new Map(hits.map((hit) => [hit.id, hit.snippet]))))
        .catch(() => setSnippets(new Map()));
    }, 200);
    return () => window.clearTimeout(timer);
  }, [search, documents]);

  useEffect(() => {
    if (view !== "inbox") refreshReport();
  }, [view, refreshReport]);

  useEffect(() => {
    if (!activeId) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setLoadingDetail(true);
    unwrap(commands.getDocument(activeId))
      .then((next) => !cancelled && setDetail(next))
      .catch((error) => !cancelled && announce(errorMessage(error), "error"))
      .finally(() => !cancelled && setLoadingDetail(false));
    return () => {
      cancelled = true;
    };
  }, [activeId, announce]);

  function replaceSummary(summary: DocumentSummary) {
    setDocuments((current) =>
      current.map((document) => (document.id === summary.id ? summary : document)),
    );
  }

  async function importFiles(files: FileList | File[]) {
    const list = Array.from(files);
    const supported = list.filter((file) => SUPPORTED.test(file.name) && file.size <= MAX_BYTES);
    const skipped = list.length - supported.length;
    if (supported.length === 0) {
      announce("Choose PDF, PNG, JPG, or TIFF files up to 25 MB.", "error");
      return;
    }
    setView("inbox");
    setQueueFilter("all");
    let added = 0;
    let duplicates = 0;
    const failures: string[] = [];
    let firstId: string | null = null;
    for (const [index, file] of supported.entries()) {
      setBusy(
        supported.length === 1
          ? `Adding ${file.name}…`
          : `Adding ${index + 1} of ${supported.length}: ${file.name}`,
      );
      try {
        const outcome = await importFile(file);
        firstId ??= outcome.document.id;
        if (outcome.duplicate) duplicates += 1;
        else added += 1;
      } catch (error) {
        failures.push(`${file.name}: ${errorMessage(error)}`);
      }
    }
    setBusy(null);
    if (fileInput.current) fileInput.current.value = "";
    if (folderInput.current) folderInput.current.value = "";
    await refreshList();
    if (firstId) setActiveId(firstId);

    const parts = [];
    if (added) parts.push(`${added} added — reading in the background`);
    if (duplicates) parts.push(`${duplicates} already in your inbox`);
    if (skipped) parts.push(`${skipped} skipped (unsupported or over 25 MB)`);
    if (failures.length) {
      announce(`${parts.length ? `${parts.join(", ")}. ` : ""}${failures.join(" · ")}`, "error");
    } else {
      announce(`${parts.join(", ")}.`);
    }
  }

  function handleFileChange(event: ChangeEvent<HTMLInputElement>) {
    if (event.target.files) void importFiles(event.target.files);
  }

  async function update(edit: FieldEdit): Promise<boolean> {
    if (!detail) return false;
    try {
      const next = await unwrap(commands.updateField(detail.summary.id, edit));
      setDetail(next);
      replaceSummary(next.summary);
      return true;
    } catch (error) {
      announce(errorMessage(error), "error");
      return false;
    }
  }

  function move(step: number) {
    if (filteredDocuments.length === 0) return;
    const index = filteredDocuments.findIndex((document) => document.id === activeId);
    const next =
      index === -1 ? 0 : Math.min(Math.max(index + step, 0), filteredDocuments.length - 1);
    const id = filteredDocuments[next].id;
    setActiveId(id);
    document.querySelector(`[data-record="${id}"]`)?.scrollIntoView({ block: "nearest" });
  }

  /** Approves any document (the board's drop target); the backend validates it as usual. */
  async function approveById(id: string) {
    try {
      const summary = await unwrap(commands.approveDocument(id));
      replaceSummary(summary);
      if (detail?.summary.id === id) setDetail({ ...detail, summary });
      announce(`${displayName(summary)} approved.`);
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function approveSelected() {
    if (!detail || detail.summary.status === "approved") return;
    try {
      const summary = await unwrap(commands.approveDocument(detail.summary.id));
      replaceSummary(summary);
      setDetail({ ...detail, summary });
      announce(`${displayName(summary)} approved.`);
      move(1);
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function reprocessSelected() {
    if (!detail) return;
    try {
      const summary = await unwrap(commands.reprocessDocument(detail.summary.id));
      replaceSummary(summary);
      setDetail(await unwrap(commands.getDocument(summary.id)));
      announce(
        summary.status === "approved"
          ? "Approved documents are kept as they are."
          : "Queued to be read again. Your corrections are kept.",
      );
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function removeSelected() {
    if (!detail) return;
    const id = detail.summary.id;
    const index = filteredDocuments.findIndex((document) => document.id === id);
    try {
      await unwrap(commands.deleteDocument(id));
      const remaining = filteredDocuments.filter((document) => document.id !== id);
      setDocuments((current) => current.filter((document) => document.id !== id));
      setActiveId(remaining[Math.min(index, remaining.length - 1)]?.id ?? null);
      announce(`${displayName(detail.summary)} deleted.`);
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function cancelSelected() {
    if (!detail) return;
    const id = detail.summary.id;
    try {
      await unwrap(commands.cancelImport(id));
      const list = await refreshList();
      if (list.some((document) => document.id === id)) {
        setDetail(await unwrap(commands.getDocument(id)));
        announce("Reading cancelled. The document's values are kept for review.");
      } else {
        setActiveId(list[0]?.id ?? null);
        announce(`Import of ${detail.summary.fileName} cancelled.`);
      }
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function cancelPending() {
    try {
      const cancelled = await unwrap(commands.cancelPendingImports());
      const list = await refreshList();
      if (activeId && !list.some((document) => document.id === activeId)) {
        setActiveId(list[0]?.id ?? null);
      }
      announce(
        cancelled === 0
          ? "Nothing was waiting to be read."
          : `${cancelled} import${cancelled === 1 ? "" : "s"} cancelled.`,
      );
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function chooseWatchFolder() {
    try {
      const status = await unwrap(commands.chooseWatchFolder());
      if (!status) return;
      setWatch(status);
      announce(`Watching ${folderName(status.folder)}. New files will appear in the inbox.`);
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function setWatching(enabled: boolean) {
    try {
      setWatch(await unwrap(commands.setWatchEnabled(enabled)));
      announce(enabled ? "Watching resumed." : "Watching paused. Files already imported stay in the inbox.");
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function forgetWatch() {
    try {
      setWatch(await unwrap(commands.forgetWatchFolder()));
      announce("Stopped watching. Files already imported stay in the inbox.");
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  function scanNow() {
    void commands.scanWatchFolder();
    announce("Checking the folder for new files…");
  }

  async function approveReady() {
    try {
      const approved = await unwrap(commands.approveReady());
      await refreshList();
      if (activeId) setDetail(await unwrap(commands.getDocument(activeId)));
      announce(
        approved === 0
          ? "Nothing was ready to approve."
          : `${approved} document${approved === 1 ? "" : "s"} approved.`,
      );
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  async function saveSettings(next: Settings): Promise<boolean> {
    try {
      const saved = await unwrap(commands.updateSettings(next));
      setSettings(saved);
      announce("Settings saved.");
      return true;
    } catch (error) {
      announce(errorMessage(error), "error");
      return false;
    }
  }

  function openVendor(name: string) {
    setView("inbox");
    setQueueFilter("all");
    setSearch(name);
  }

  function openOriginal() {
    if (!detail) return;
    unwrap(commands.openDocumentFile(detail.summary.id)).catch((error) =>
      announce(errorMessage(error), "error"),
    );
  }

  async function runAsk(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const text = question.trim();
    if (!text || asking) return;
    const index = entries.length;
    setEntries((current) => [...current, { question: text }]);
    setQuestion("");
    setAsking(true);
    try {
      const answer = await unwrap(commands.ask(text));
      setEntries((current) => current.map((entry, i) => (i === index ? { ...entry, answer } : entry)));
    } catch (error) {
      setEntries((current) =>
        current.map((entry, i) => (i === index ? { ...entry, error: errorMessage(error) } : entry)),
      );
    } finally {
      setAsking(false);
    }
  }

  /** Step one: show what would be written, and where. Nothing is written yet. */
  async function exportCsv(format: ExportFormat) {
    try {
      const plan = await unwrap(commands.exportPlan(format));
      setExportState({ format, plan });
    } catch (error) {
      announce(errorMessage(error), "error");
    }
  }

  /** Step two: the person confirmed the path. */
  async function confirmExport() {
    if (!exportState) return;
    setExporting(true);
    try {
      const result = await unwrap(commands.exportRecords(exportState.format));
      setExportState(null);
      const what = exportState.format === "ofx" ? "transaction" : "record";
      const where = result.paths.length === 1 ? result.paths[0] : `${result.paths.length} files`;
      announce(`Exported ${result.rows} ${what}${result.rows === 1 ? "" : "s"} to ${where}`);
    } catch (error) {
      announce(errorMessage(error), "error");
    } finally {
      setExporting(false);
    }
  }

  function showApproved() {
    setView("inbox");
    setQueueFilter("approved");
  }

  const paletteCommands: Command[] = [
    ...navItems.map(({ id, label, icon: Icon, key }) => ({
      id: `go-${id}`,
      label: `Go to ${label}`,
      icon: <Icon size={15} />,
      shortcut: key,
      run: () => setView(id),
    })),
    {
      id: "add",
      label: "Add documents",
      icon: <Upload size={15} />,
      shortcut: "N",
      run: () => fileInput.current?.click(),
    },
    ...(canImportFolder
      ? [
          {
            id: "folder",
            label: "Import a folder",
            icon: <FolderInput size={15} />,
            run: () => folderInput.current?.click(),
          },
        ]
      : []),
    ...(canWatch
      ? [
          {
            id: "watch",
            label: watch?.folder ? "Change the watched folder" : "Watch a folder for new documents",
            icon: <Eye size={15} />,
            run: () => void chooseWatchFolder(),
          },
        ]
      : []),
    ...(canWatch && watch?.folder
      ? [
          {
            id: "watch-toggle",
            label: watch.enabled ? "Pause folder watching" : "Resume folder watching",
            icon: <Eye size={15} />,
            run: () => void setWatching(!watch.enabled),
          },
        ]
      : []),
    ...(activity && activity.queued > 0
      ? [
          {
            id: "cancel-pending",
            label: `Cancel ${activity.queued} waiting import${activity.queued === 1 ? "" : "s"}`,
            icon: <X size={15} />,
            run: () => void cancelPending(),
          },
        ]
      : []),
    {
      id: "approve-ready",
      label: "Approve all ready documents",
      icon: <CheckCheck size={15} />,
      run: () => void approveReady(),
    },
    {
      id: "settings",
      label: "Open settings",
      icon: <Settings2 size={15} />,
      shortcut: ",",
      run: () => setView("settings"),
    },
    {
      id: "shortcuts",
      label: "Keyboard shortcuts",
      icon: <Keyboard size={15} />,
      shortcut: "?",
      run: () => setShortcutsOpen(true),
    },
    {
      id: "review",
      label: "Show documents that need review",
      icon: <ListFilter size={15} />,
      run: () => {
        setView("inbox");
        setQueueFilter("review");
      },
    },
    {
      id: "approved",
      label: "Show approved records",
      icon: <CheckCircle2 size={15} />,
      run: showApproved,
    },
    {
      id: "export",
      label: "Export reviewed records as CSV",
      icon: <ArrowDownToLine size={15} />,
      run: () => void exportCsv("canonical"),
    },
    {
      id: "theme",
      label: theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      icon: <Moon size={15} />,
      run: () => setTheme(theme === "dark" ? "light" : "dark"),
    },
  ];

  // Re-bound every render so handlers see current state.
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((open) => !open);
        return;
      }
      if (paletteOpen || shortcutsOpen || exportState) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (isTyping(event.target)) return;
      if (event.key === "?") {
        setShortcutsOpen(true);
        return;
      }
      if (event.key === ",") {
        setView("settings");
        return;
      }
      const nav = navItems.find((item) => item.key === event.key);
      if (nav) {
        setView(nav.id);
        return;
      }
      if (event.key === "n") {
        fileInput.current?.click();
        return;
      }
      if (view !== "inbox") return;
      if (event.key === "j" || event.key === "ArrowDown") {
        event.preventDefault();
        move(1);
      } else if (event.key === "k" || event.key === "ArrowUp") {
        event.preventDefault();
        move(-1);
      } else if (event.key === "a") {
        void approveSelected();
      } else if (event.key === "e") {
        event.preventDefault();
        vendorRef.current?.focus();
        vendorRef.current?.select();
      } else if (event.key === "/") {
        event.preventDefault();
        searchRef.current?.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div className="app-shell">
      <Rail
        view={view}
        setView={setView}
        reviewCount={reviewCount}
        showApproved={showApproved}
        showShortcuts={() => setShortcutsOpen(true)}
        workspaceName={workspaceName}
        theme={theme}
        toggleTheme={() => setTheme(theme === "dark" ? "light" : "dark")}
      />

      <main className="main">
        <header className="topbar">
          <div className="crumbs">
            <span className="crumb-workspace">{workspaceName}</span>
            <h1>
              {titles[view]}
              {view === "inbox" && reviewCount > 0 && (
                <span className="tag review">{reviewCount} to review</span>
              )}
            </h1>
            <p>{subtitles[view]}</p>
          </div>
          <div className="topbar-tools">
            <button className="palette-trigger" onClick={() => setPaletteOpen(true)}>
              <Search size={14} aria-hidden="true" />
              <span>Search or run a command</span>
              <kbd>
                <CommandIcon size={11} aria-hidden="true" />K
              </kbd>
            </button>
            <input
              ref={fileInput}
              className="visually-hidden"
              type="file"
              multiple
              accept=".pdf,.png,.jpg,.jpeg,.tif,.tiff,application/pdf,image/png,image/jpeg,image/tiff"
              onChange={handleFileChange}
              tabIndex={-1}
            />
            <input
              ref={folderInput}
              className="visually-hidden"
              type="file"
              multiple
              {...{ webkitdirectory: "" }}
              onChange={handleFileChange}
              tabIndex={-1}
            />
            {canImportFolder && (
              <button
                className="button icon-only"
                onClick={() => folderInput.current?.click()}
                disabled={busy !== null}
                aria-label="Import a folder"
                title="Import a folder"
              >
                <FolderInput size={15} aria-hidden="true" />
              </button>
            )}
            <button
              className="button button-primary"
              onClick={() => fileInput.current?.click()}
              disabled={busy !== null}
            >
              <Plus size={15} aria-hidden="true" /> Add <kbd>N</kbd>
            </button>
          </div>
        </header>

        <div className="content">
          {view === "inbox" && (
            loaded && documents.length === 0 && !welcomeSkipped ? (
              <Welcome
                ocrAvailable={info?.ocrAvailable ?? null}
                addDocuments={() => fileInput.current?.click()}
                importFolder={canImportFolder ? () => folderInput.current?.click() : null}
                watchFolder={canWatch ? () => void chooseWatchFolder() : null}
                openSettings={() => setView("settings")}
                skip={skipWelcome}
              />
            ) : loaded ? (
              <InboxView
                documents={documents}
                filteredDocuments={filteredDocuments}
                activeId={activeId}
                filter={queueFilter}
                setFilter={setQueueFilter}
                search={search}
                setSearch={setSearch}
                searchRef={searchRef}
                select={setActiveId}
                detail={detail}
                loadingDetail={loadingDetail}
                categories={categories}
                update={update}
                approve={() => void approveSelected()}
                skip={() => move(1)}
                reprocess={() => void reprocessSelected()}
                remove={() => void removeSelected()}
                cancelImport={() => void cancelSelected()}
                layout={inboxLayout}
                setLayout={setInboxLayout}
                approveById={(id) => void approveById(id)}
                cancelPending={() => void cancelPending()}
                activity={activity}
                watch={watch}
                openWatchSettings={() => setView("settings")}
                openOriginal={canOpenOriginal ? openOriginal : null}
                vendorRef={vendorRef}
                dragging={dragging}
                setDragging={setDragging}
                importFiles={(files) => void importFiles(files)}
                chooseFiles={() => fileInput.current?.click()}
                chooseFolder={canImportFolder ? () => folderInput.current?.click() : null}
                busy={busy}
                sort={sort}
                setSort={setSort}
                approveReady={() => void approveReady()}
                snippets={snippets}
              />
            ) : (
              <div className="loading-state">Opening workspace…</div>
            )
          )}
          {view === "ask" && (
            <AskView
              question={question}
              setQuestion={setQuestion}
              entries={entries}
              runAsk={(event) => void runAsk(event)}
              asking={asking}
              report={report}
            />
          )}
          {view === "reports" && (
            <ReportsView
              report={report}
              exportCsv={(format) => void exportCsv(format)}
              workspaceName={workspaceName}
              since={documents.length ? Math.min(...documents.map((document) => document.importedAt)) : null}
              defaultCurrency={settings?.defaultCurrency ?? null}
            />
          )}
          {view === "vendors" && <VendorsView vendors={vendors} openVendor={openVendor} />}
          {view === "settings" && (
            <SettingsView
              settings={settings}
              info={info}
              save={saveSettings}
              theme={theme}
              setTheme={setTheme}
              showShortcuts={() => setShortcutsOpen(true)}
              watch={watch}
              watching={activity?.watching ?? false}
              scanning={activity?.scanning ?? false}
              chooseWatchFolder={() => void chooseWatchFolder()}
              folderWatching={canWatch}
              setWatching={(enabled) => void setWatching(enabled)}
              forgetWatch={() => void forgetWatch()}
              scanNow={scanNow}
            />
          )}
        </div>

        <footer className="statusbar">
          <span className="status-local">
            <ShieldCheck size={12} aria-hidden="true" />
            Local · stored on this device
          </span>
          {watch?.folder && watch.enabled && (
            <button className="status-watch" onClick={() => setView("settings")} title={watch.folder}>
              <Eye size={12} aria-hidden="true" /> Watching {folderName(watch.folder)}
            </button>
          )}
          {activity && (activity.queued > 0 || activity.reading) && (
            <span className="status-reading" role="status">
              Reading{activity.queued > 0 ? ` · ${activity.queued} waiting` : "…"}
            </span>
          )}
          <span className="mono">
            {documents.length} document{documents.length === 1 ? "" : "s"}
          </span>
          <span className="statusbar-keys">
            {view === "inbox" ? (
              <>
                <span><kbd>J</kbd><kbd>K</kbd> move</span>
                <span><kbd>A</kbd> approve</span>
                <span><kbd>E</kbd> edit</span>
                <span><kbd>/</kbd> filter</span>
              </>
            ) : (
              <span><kbd>1</kbd><kbd>2</kbd><kbd>3</kbd><kbd>4</kbd> switch view</span>
            )}
            {view !== "settings" && (
              <span><kbd>?</kbd> shortcuts</span>
            )}
            <span><kbd>⌘K</kbd> commands</span>
          </span>
        </footer>
      </main>

      <nav className="mobile-nav" aria-label="Primary navigation">
        {navItems.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            className={view === id ? "active" : ""}
            onClick={() => setView(id)}
            aria-current={view === id ? "page" : undefined}
          >
            <Icon size={17} strokeWidth={1.7} aria-hidden="true" />
            <span>{label}</span>
          </button>
        ))}
      </nav>

      {paletteOpen && (
        <CommandPalette
          commands={paletteCommands}
          records={documents}
          openRecord={(id) => {
            setView("inbox");
            setQueueFilter("all");
            setSearch("");
            setActiveId(id);
          }}
          close={() => setPaletteOpen(false)}
        />
      )}

      {shortcutsOpen && <ShortcutsSheet onClose={() => setShortcutsOpen(false)} />}

      {exportState && (
        <ExportDialog
          format={exportState.format}
          plan={exportState.plan}
          busy={exporting}
          onConfirm={() => void confirmExport()}
          onClose={() => setExportState(null)}
        />
      )}

      {toast && (
        <div className={`toast ${toast.tone}`} role={toast.tone === "error" ? "alert" : "status"}>
          {toast.tone === "error" ? (
            <CircleAlert size={15} aria-hidden="true" />
          ) : (
            <CheckCircle2 size={15} aria-hidden="true" />
          )}
          <span>{toast.message}</span>
          <button aria-label="Dismiss notification" onClick={() => setToast(null)}>
            <X size={14} />
          </button>
        </div>
      )}
    </div>
  );
}

export default App;
