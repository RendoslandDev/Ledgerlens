import {
  ArrowRight,
  Building2,
  ChevronRight,
  Eye,
  FileText,
  FolderInput,
  FolderOpen,
  Image,
  Receipt,
  RefreshCw,
  ScanText,
  Settings2,
  ShieldCheck,
  Sparkles,
  ThumbsUp,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { LogoMark } from "./Logo";

type Tile = { icon: LucideIcon; area: string };

type Start = {
  id: string;
  title: string;
  description: string;
  tag: { icon: LucideIcon; label: string };
  /** The tile layout previews what the choice does, like a board template. */
  layout: string;
  tiles: Tile[];
  run: () => void;
};

/**
 * First-run screen for an empty workspace: each card starts a real action (nothing here creates
 * sample data). Skipping opens the empty inbox.
 */
export function Welcome({
  ocrAvailable,
  addDocuments,
  importFolder,
  watchFolder,
  openSettings,
  skip,
}: {
  ocrAvailable: boolean | null;
  addDocuments: () => void;
  /** `null` where the device can't do it (phones have no folder picker). */
  importFolder: (() => void) | null;
  watchFolder: (() => void) | null;
  openSettings: () => void;
  skip: () => void;
}) {
  const all: (Omit<Start, "run"> & { run: (() => void) | null })[] = [
    {
      id: "add",
      title: "Add documents",
      description: "Pick invoices and receipts to read",
      tag: { icon: ThumbsUp, label: "Recommended" },
      layout: "a b / c d",
      tiles: [
        { icon: FileText, area: "a" },
        { icon: Image, area: "b" },
        { icon: Receipt, area: "c" },
        { icon: ScanText, area: "d" },
      ],
      run: addDocuments,
    },
    {
      id: "folder",
      title: "Import a folder",
      description: "Bring in everything saved in one place",
      tag: { icon: Zap, label: "Bulk" },
      layout: "a a / b c",
      tiles: [
        { icon: FolderInput, area: "a" },
        { icon: FileText, area: "b" },
        { icon: Receipt, area: "c" },
      ],
      run: importFolder,
    },
    {
      id: "watch",
      title: "Watch a folder",
      description: "New files are added as they appear",
      tag: { icon: Sparkles, label: "Automatic" },
      layout: "a b / a c",
      tiles: [
        { icon: FolderOpen, area: "a" },
        { icon: Eye, area: "b" },
        { icon: RefreshCw, area: "c" },
      ],
      run: watchFolder,
    },
    {
      id: "setup",
      title: "Set up workspace",
      description: "Name, currency, and review threshold",
      tag: { icon: Settings2, label: "Setup" },
      layout: "a b / c c",
      tiles: [
        { icon: Settings2, area: "a" },
        { icon: Building2, area: "b" },
        { icon: ShieldCheck, area: "c" },
      ],
      run: openSettings,
    },
  ];
  const starts = all.filter((start): start is Start => start.run !== null);

  return (
    <section className="welcome" aria-labelledby="welcome-title">
      <div className="welcome-head">
        <span className="welcome-logo">
          <LogoMark size={34} />
        </span>
        <h1 id="welcome-title">Welcome to Ledgerlens</h1>
        <p>
          Add your paperwork or choose how to start.{" "}
          {ocrAvailable === false
            ? "PDFs with text are read on this device."
            : "PDFs, scans, and photos are read on this device."}
        </p>
      </div>

      <div className="welcome-grid">
        {starts.map((start) => (
          <button key={start.id} className="welcome-card" onClick={start.run}>
            <span className="welcome-tiles" style={{ gridTemplateAreas: areas(start.layout) }} aria-hidden="true">
              {start.tiles.map(({ icon: Icon, area }) => (
                <span key={area} style={{ gridArea: area }}>
                  <Icon size={20} strokeWidth={1.6} />
                </span>
              ))}
            </span>
            <span className="welcome-copy">
              <strong>{start.title}</strong>
              <span>{start.description}</span>
            </span>
            <span className="welcome-foot">
              <span className="tag">
                <start.tag.icon size={12} aria-hidden="true" /> {start.tag.label}
              </span>
              <span className="welcome-go">
                Start <ChevronRight size={14} aria-hidden="true" />
              </span>
            </span>
          </button>
        ))}
      </div>

      <div className="welcome-skip">
        <span>Or skip and start with an empty inbox</span>
        <button className="button" onClick={skip}>
          Open empty inbox <ArrowRight size={14} aria-hidden="true" />
        </button>
      </div>
    </section>
  );
}

/** "a b / c d" -> CSS grid-template-areas. */
function areas(layout: string): string {
  return layout
    .split("/")
    .map((row) => `"${row.trim()}"`)
    .join(" ");
}
