import { useCallback, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { Layers, Plus } from "lucide-react";
import { api, type Job, type Stats } from "../lib/api";
import { bytes, dateTime, STATUS_LABEL } from "../lib/format";
import { useProgress } from "../lib/useProgress";

const DAY = 86400000;

function lastSevenDays(rows: Stats["done_last_7_days"]) {
  const map = new Map(rows.map((r) => [r.date, r.count]));
  const out: { date: string; label: string; count: number }[] = [];
  for (let i = 6; i >= 0; i--) {
    const d = new Date(Date.now() - i * DAY);
    const key = d.toISOString().slice(0, 10);
    out.push({
      date: key,
      label: d.toLocaleDateString("pt-BR", { weekday: "short" }).replace(".", ""),
      count: map.get(key) ?? 0,
    });
  }
  return out;
}

function ProductionChart({ days }: { days: ReturnType<typeof lastSevenDays> }) {
  const max = Math.max(4, ...days.map((d) => d.count));
  const total = days.reduce((s, d) => s + d.count, 0);
  return (
    <div className="card" style={{ gridColumn: "span 2" }}>
      <div className="card-head">
        <span className="card-title">Vídeos prontos por dia</span>
        <span className="meta mono">{total} nos últimos 7 dias</span>
      </div>
      <div className="card-pad" style={{ display: "grid", gridTemplateColumns: "40px 1fr", gap: 12, height: 320 }}>
        <div className="mono muted" style={{ display: "flex", flexDirection: "column", justifyContent: "space-between", fontSize: 12, textAlign: "right", paddingBottom: 28 }}>
          <span>{max}</span>
          <span>{Math.round(max / 2)}</span>
          <span>0</span>
        </div>
        <div style={{ display: "grid", gridTemplateRows: "1fr 28px" }}>
          <div style={{
            display: "grid", gridTemplateColumns: "repeat(7, 1fr)", gap: 16, alignItems: "end",
            backgroundImage: "linear-gradient(var(--line) 1px, transparent 1px)", backgroundSize: "100% 50%",
            borderBottom: "1px solid var(--line-2)",
          }}>
            {days.map((d, i) => (
              <div key={d.date} title={`${d.count} vídeos`} style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 6, height: "100%", justifyContent: "flex-end" }}>
                {d.count > 0 && <span className="mono" style={{ fontSize: 12, color: i === 6 ? "var(--text)" : "var(--text-3)" }}>{d.count}</span>}
                <div style={{
                  width: "100%", maxWidth: 56,
                  height: `${(d.count / max) * 100}%`, minHeight: d.count ? 4 : 0,
                  background: i === 6 ? "var(--accent)" : "var(--raised-2)",
                  borderRadius: "4px 4px 0 0",
                }} />
              </div>
            ))}
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(7, 1fr)", gap: 16 }}>
            {days.map((d, i) => (
              <span key={d.date} className="mono" style={{ fontSize: 12, textAlign: "center", alignSelf: "end", color: i === 6 ? "var(--text-2)" : "var(--text-3)" }}>
                {i === 6 ? "hoje" : d.label}
              </span>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

export function Inicio() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);

  const load = useCallback(() => {
    api.get<Stats>("/stats").then(setStats, () => {});
    api.get<Job[]>("/jobs?limit=6").then(setJobs, () => {});
  }, []);
  useEffect(load, [load]);

  // Recarrega quando um item termina, sem martelar a API a cada porcento.
  useProgress((e) => { if (e.status !== "processing") load(); });

  const s = stats?.items_by_status ?? {};
  const days = useMemo(() => lastSevenDays(stats?.done_last_7_days ?? []), [stats]);
  const disk = stats?.disk_bytes;
  const diskTotal = disk ? disk.uploads + disk.output + disk.templates : 0;
  const m = new Date().toLocaleDateString("pt-BR", { month: "long", year: "numeric" }).replace(" de ", " ");
  const month = m.charAt(0).toUpperCase() + m.slice(1);

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">{month}</h1>
          <p className="page-sub">Produção da estação</p>
        </div>
        <div className="page-actions">
          <Link to="/lotes" className="btn btn-primary"><Plus size={16} strokeWidth={1.5} />Novo lote</Link>
        </div>
      </header>

      <section className="stats">
        {([
          ["Na fila", s.queued ?? 0, "aguardando render"],
          ["Renderizando", s.processing ?? 0, "agora"],
          ["Prontos", s.done ?? 0, "desde o início"],
          ["Falhas", s.error ?? 0, "com log disponível"],
        ] as const).map(([label, value, note]) => (
          <div key={label} className="card stat">
            <span className="stat-label">{label}</span>
            <span className="stat-value">{value.toLocaleString("pt-BR")}</span>
            <span className="stat-note">{note}</span>
          </div>
        ))}
      </section>

      <section style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: 12 }}>
        <ProductionChart days={days} />
        <div className="card">
          <div className="card-head">
            <span className="card-title">Disco</span>
            <span className="meta mono">{bytes(diskTotal)}</span>
          </div>
          <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {disk && ([
              ["Vídeos prontos", disk.output],
              ["Uploads", disk.uploads],
              ["Fundos de modelo", disk.templates],
            ] as const).map(([label, n]) => (
              <div key={label} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <div className="row" style={{ justifyContent: "space-between" }}>
                  <span className="dim">{label}</span>
                  <span className="mono muted" style={{ fontSize: 12 }}>{bytes(n)}</span>
                </div>
                <div className="progress"><i style={{ width: `${diskTotal ? (n / diskTotal) * 100 : 0}%`, background: "var(--raised-2)" }} /></div>
              </div>
            ))}
            <span className="muted" style={{ fontSize: 12 }}>{stats?.templates ?? 0} modelos de fundo salvos</span>
          </div>
        </div>
      </section>

      <section className="card">
        <div className="card-head">
          <span className="card-title">Lotes recentes</span>
          <Link to="/lotes" className="meta">Ver todos</Link>
        </div>
        {jobs.length === 0 ? (
          <div className="empty">
            <Layers size={20} strokeWidth={1.5} />
            Nenhum lote ainda
          </div>
        ) : (
          <div className="list">
            {jobs.map((j) => {
              const done = j.items.filter((i) => i.status === "done").length;
              const pct = j.items.length ? (j.items.reduce((s, i) => s + (i.status === "done" ? 100 : i.progress), 0) / j.items.length) : 0;
              return (
                <div key={j.id} className="list-row" style={{ gridTemplateColumns: "80px 1fr 200px 72px 240px" }}>
                  <span className="mono muted">#{String(j.id).padStart(4, "0")}</span>
                  <span>{j.items.length} vídeos</span>
                  <div className={`progress ${j.status === "done" ? "done" : j.status === "error" ? "err" : ""}`}><i style={{ width: `${pct}%` }} /></div>
                  <span className="mono dim" style={{ textAlign: "right" }}>{done}/{j.items.length}</span>
                  <span className="dim" style={{ textAlign: "right" }}>{STATUS_LABEL[j.status]} · <span className="mono muted">{dateTime(j.created_at)}</span></span>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </div>
  );
}
