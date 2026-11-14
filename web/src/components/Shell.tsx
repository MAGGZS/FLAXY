import { useEffect, useState } from "react";
import { NavLink, Outlet } from "react-router-dom";
import {
  Bot, Clapperboard, FolderOpen, Gauge, Layers, LayoutTemplate, Scissors, Settings,
} from "lucide-react";
import { api, type Health } from "../lib/api";

const ICON = { size: 16, strokeWidth: 1.5 };

type Item = { to: string; label: string; icon: React.ReactNode; off?: string };

const PRODUCE: Item[] = [
  { to: "/", label: "Início", icon: <Gauge {...ICON} /> },
  { to: "/lotes", label: "Lotes", icon: <Layers {...ICON} /> },
  { to: "/clipes", label: "Clipes", icon: <Scissors {...ICON} /> },
  { to: "/biblioteca", label: "Biblioteca", icon: <FolderOpen {...ICON} /> },
];

const SETUP: Item[] = [
  { to: "/modelos", label: "Modelos", icon: <LayoutTemplate {...ICON} /> },
  { to: "/ia", label: "IA", icon: <Bot {...ICON} /> },
  { to: "/configuracoes", label: "Configurações", icon: <Settings {...ICON} /> },
];

function Nav({ label, items }: { label: string; items: Item[] }) {
  return (
    <nav className="nav-group" aria-label={label}>
      <div className="nav-label">{label}</div>
      {items.map((i) =>
        i.off ? (
          <span key={i.to} className="nav-item off" aria-disabled="true">
            {i.icon}
            {i.label}
            <span className="meta mono">{i.off}</span>
          </span>
        ) : (
          <NavLink key={i.to} to={i.to} end={i.to === "/"} className={({ isActive }) => `nav-item${isActive ? " active" : ""}`}>
            {i.icon}
            {i.label}
          </NavLink>
        ),
      )}
    </nav>
  );
}

export function Shell() {
  const [health, setHealth] = useState<Health | null>(null);
  const [down, setDown] = useState(false);

  useEffect(() => {
    let alive = true;
    const check = () =>
      api.get<Health>("/health").then(
        (h) => alive && (setHealth(h), setDown(false)),
        () => alive && setDown(true),
      );
    check();
    const t = setInterval(check, 15000);
    return () => { alive = false; clearInterval(t); };
  }, []);

  return (
    <div className="shell">
      <aside className="side">
        <div className="brand">
          <Clapperboard size={18} strokeWidth={1.5} color="var(--accent-ink)" />
          <span className="brand-name">FLAXY</span>
          <span className="brand-ver mono muted">{health ? `v${health.version}` : ""}</span>
        </div>
        <Nav label="Produção" items={PRODUCE} />
        <Nav label="Ajustes" items={SETUP} />
        <div className="side-foot">
          <div className="row" style={{ gap: 8 }}>
            <span className={`status-dot ${down ? "err" : health ? "ok" : ""}`} />
            <span className="dim">{down ? "Servidor fora do ar" : health ? "Servidor no ar" : "Conectando"}</span>
          </div>
          {health && (
            <span className="mono muted">
              {health.encoder} · {health.ffmpeg ? "ffmpeg ok" : "sem ffmpeg"}
            </span>
          )}
        </div>
      </aside>
      <main className="main">
        <Outlet />
      </main>
    </div>
  );
}
