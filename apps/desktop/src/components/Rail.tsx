import {
  Building2,
  CircleHelp,
  Inbox,
  Keyboard,
  LayoutDashboard,
  Moon,
  Settings2,
  Sun,
  WalletCards,
} from "lucide-react";
import { LogoMark } from "./Logo";

export type View = "inbox" | "ask" | "reports" | "vendors" | "settings";

export const navItems: {
  id: View;
  label: string;
  icon: typeof Inbox;
  key: string;
}[] = [
  { id: "inbox", label: "Inbox", icon: Inbox, key: "1" },
  { id: "ask", label: "Ask", icon: CircleHelp, key: "2" },
  { id: "reports", label: "Reports", icon: LayoutDashboard, key: "3" },
  { id: "vendors", label: "Vendors", icon: Building2, key: "4" },
];

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2);
  return letters.toUpperCase();
}

export function Rail({
  view,
  setView,
  reviewCount,
  showApproved,
  showShortcuts,
  workspaceName,
  theme,
  toggleTheme,
}: {
  view: View;
  setView: (view: View) => void;
  reviewCount: number;
  showApproved: () => void;
  showShortcuts: () => void;
  workspaceName: string;
  theme: "dark" | "light";
  toggleTheme: () => void;
}) {
  const primary = navItems.filter((item) => item.id !== "vendors");
  return (
    <aside className="rail">
      <a
        className="rail-logo"
        href="#inbox"
        onClick={() => setView("inbox")}
        aria-label="Ledgerlens home"
      >
        <LogoMark />
      </a>
      <nav className="rail-group" aria-label="Primary navigation">
        {primary.map(({ id, label, icon: Icon, key }) => (
          <button
            key={id}
            className={`rail-button ${view === id ? "active" : ""}`}
            onClick={() => setView(id)}
            aria-current={view === id ? "page" : undefined}
            aria-label={label}
            data-tip={`${label}  ${key}`}
          >
            <Icon size={18} strokeWidth={1.7} aria-hidden="true" />
            {id === "inbox" && reviewCount > 0 && (
              <span className="rail-badge">{reviewCount}</span>
            )}
          </button>
        ))}
      </nav>
      <div className="rail-divider" />
      <div className="rail-group">
        <button
          className="rail-button"
          onClick={showApproved}
          aria-label="Approved records"
          data-tip="Approved records"
        >
          <WalletCards size={18} strokeWidth={1.7} aria-hidden="true" />
        </button>
        <button
          className={`rail-button ${view === "vendors" ? "active" : ""}`}
          onClick={() => setView("vendors")}
          aria-current={view === "vendors" ? "page" : undefined}
          aria-label="Vendors"
          data-tip="Vendors  4"
        >
          <Building2 size={18} strokeWidth={1.7} aria-hidden="true" />
        </button>
      </div>
      <div className="rail-bottom">
        <button
          className="rail-button"
          onClick={showShortcuts}
          aria-label="Keyboard shortcuts"
          data-tip="Shortcuts  ?"
        >
          <Keyboard size={17} strokeWidth={1.7} aria-hidden="true" />
        </button>
        <button
          className="rail-button"
          onClick={toggleTheme}
          aria-label={theme === "dark" ? "Use light theme" : "Use dark theme"}
          data-tip={theme === "dark" ? "Light theme" : "Dark theme"}
        >
          {theme === "dark" ? (
            <Sun size={17} strokeWidth={1.7} aria-hidden="true" />
          ) : (
            <Moon size={17} strokeWidth={1.7} aria-hidden="true" />
          )}
        </button>
        <button
          className={`rail-button ${view === "settings" ? "active" : ""}`}
          onClick={() => setView("settings")}
          aria-current={view === "settings" ? "page" : undefined}
          aria-label="Settings"
          data-tip="Settings  ,"
        >
          <Settings2 size={17} strokeWidth={1.7} aria-hidden="true" />
        </button>
        <button
          className="rail-avatar"
          onClick={() => setView("settings")}
          title={`${workspaceName} · stored on this device`}
          aria-label={`Workspace: ${workspaceName}`}
        >
          {initials(workspaceName)}
        </button>
      </div>
    </aside>
  );
}
