import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { Check, Crop, Download, Link2, Play, Scissors, Trash2 } from "lucide-react";
import { api, apiUrl, basename, type Clip, type ClipLayout, type ClipProject, type ClipProjectView, type Rect, type StyleRow } from "../lib/api";
import { dateTime, timecode } from "../lib/format";
import { useToast } from "../components/Toast";
import { RectEditor } from "../components/RectEditor";

const ICON = { size: 16, strokeWidth: 1.5 };

const RANGES: [string, number, number][] = [
  ["15–30s", 15, 30],
  ["30–60s", 30, 60],
  ["60–90s", 60, 90],
  ["Automático", 15, 120],
];

const LAYOUTS: [ClipLayout, string][] = [
  ["vertical_camera", "Vertical · câmera"],
  ["vertical_split", "Vertical · tela + câmera"],
  ["horizontal", "Horizontal"],
];

const STAGE: Record<ClipProject["status"], string> = {
  downloading: "Baixando",
  transcribing: "Transcrevendo",
  analyzing: "Analisando retenção",
  ready: "Pronto para revisar",
  error: "Falhou",
};

function suggestRects(layout: ClipLayout, w: number, h: number): { camera: Rect; screen: Rect } {
  const ar = w > 0 && h > 0 ? w / h : 16 / 9;
  const vw = Math.min(1, 9 / 16 / ar);
  const full = { x: 0, y: 0, w: 1, h: 1 };
  if (layout === "vertical_split") return { camera: { x: 0.68, y: 0.62, w: 0.3, h: 0.36 }, screen: full };
  if (layout === "horizontal") return { camera: full, screen: full };
  return { camera: { x: (1 - vw) / 2, y: 0, w: vw, h: 1 }, screen: full };
}

const busy = (p: ClipProject) => p.status === "downloading" || p.status === "transcribing" || p.status === "analyzing";

export function Clipes() {
  const toast = useToast();
  const [projects, setProjects] = useState<ClipProject[]>([]);
  const [params, setParams] = useSearchParams();
  const sel = params.get("v") ? Number(params.get("v")) : null;
  const setSel = (v: number | null | ((c: number | null) => number | null)) => {
    const next = typeof v === "function" ? v(sel) : v;
    setParams(next ? { v: String(next) } : {}, { replace: true });
  };
  const [url, setUrl] = useState("");
  const [range, setRange] = useState(1);
  const [layout, setLayout] = useState<ClipLayout>("vertical_camera");
  const [sending, setSending] = useState(false);
  const [ytdlp, setYtdlp] = useState(true);

  const loadList = useCallback(() => api.get<ClipProject[]>("/clips").then((r) => {
    setProjects(r);
    setSel((cur) => cur ?? r[0]?.id ?? null);
  }, (e) => toast("err", e.message)), [toast]);

  useEffect(() => {
    loadList();
    api.get<{ ytdlp: boolean }>("/clips/tools").then((r) => setYtdlp(r.ytdlp), () => {});
  }, [loadList]);

  // Atualiza a lista enquanto algum projeto está no pipeline.
  const anyBusy = projects.some(busy);
  useEffect(() => {
    if (!anyBusy) return;
    const t = setInterval(loadList, 2000);
    return () => clearInterval(t);
  }, [anyBusy, loadList]);

  async function create() {
    const [, min, max] = RANGES[range];
    setSending(true);
    try {
      const p = await api.post<ClipProjectView>("/clips", { url, min_len: min, max_len: max, layout });
      setUrl("");
      setSel(p.id);
      loadList();
    } catch (e: any) {
      toast("err", e.message);
    } finally {
      setSending(false);
    }
  }

  async function remove(p: ClipProject) {
    if (!confirm(`Apagar "${p.title || p.url}" e o vídeo baixado? Clipes já renderizados ficam na Biblioteca`)) return;
    await api.del(`/clips/${p.id}`).catch((e) => toast("err", e.message));
    if (sel === p.id) setSel(null);
    loadList();
  }

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">Clipes</h1>
          <p className="page-sub">Cole um vídeo longo. A IA acha os trechos que prendem e você aprova antes de cortar</p>
        </div>
      </header>

      <section className="card">
        <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="row" style={{ gap: 12 }}>
            <div className="row" style={{ flex: 1, gap: 8, background: "var(--raised)", border: "1px solid var(--line)", borderRadius: "var(--r)", padding: "0 12px", height: 44 }}>
              <Link2 {...ICON} color="var(--text-3)" />
              <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://www.youtube.com/watch?v=…" aria-label="Link do vídeo"
                onKeyDown={(e) => e.key === "Enter" && url && create()}
                style={{ flex: 1, background: "none", border: 0, outline: "none", height: "100%" }} />
            </div>
            <button className="btn btn-primary" style={{ height: 44 }} onClick={create} disabled={!url.trim() || sending || !ytdlp}>
              <Scissors {...ICON} />Encontrar clipes
            </button>
          </div>
          <div className="row" style={{ gap: 24, flexWrap: "wrap" }}>
            <div className="field"><span>Duração de cada clipe</span>
              <div className="seg">{RANGES.map(([l], i) => <button key={l} className={range === i ? "on" : ""} onClick={() => setRange(i)}>{l}</button>)}</div>
            </div>
            <div className="field"><span>Layout padrão</span>
              <div className="seg">{LAYOUTS.map(([k, l]) => <button key={k} className={layout === k ? "on" : ""} onClick={() => setLayout(k)}>{l}</button>)}</div>
            </div>
            <span className="muted" style={{ fontSize: 12, alignSelf: "flex-end", paddingBottom: 8 }}>
              {ytdlp ? <>Transcrição e LLM configurados na <Link to="/ia" style={{ color: "var(--accent-ink)" }}>aba IA</Link></> : <span style={{ color: "var(--err)" }}>yt-dlp não encontrado. Reinstale o FLAXY</span>}
            </span>
          </div>
        </div>
      </section>

      <section style={{ display: "grid", gridTemplateColumns: "280px minmax(0, 1fr)", gap: 12, alignItems: "start" }}>
        <div className="card">
          <div className="card-head"><span className="card-title">Vídeos</span><span className="meta mono">{projects.length}</span></div>
          {projects.length === 0 ? <div className="empty"><Scissors size={20} strokeWidth={1.5} />Nenhum vídeo ainda</div> : (
            <div className="list">
              {projects.map((p) => (
                <button key={p.id} className="list-row" onClick={() => setSel(p.id)}
                  style={{ gridTemplateColumns: "1fr auto", textAlign: "left", background: sel === p.id ? "var(--select)" : undefined, padding: "10px 12px 10px 20px" }}>
                  <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
                    <span style={{ fontSize: 13, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.title || p.url}</span>
                    {busy(p) ? (
                      <>
                        <span className="muted" style={{ fontSize: 12 }}>{STAGE[p.status]}{p.status === "downloading" ? <span className="mono"> {p.progress}%</span> : ""}</span>
                        <div className="progress"><i style={{ width: p.status === "downloading" ? `${p.progress}%` : "100%", opacity: p.status === "downloading" ? 1 : 0.5 }} /></div>
                      </>
                    ) : (
                      <span className="mono muted" style={{ fontSize: 12, color: p.status === "error" ? "var(--err)" : undefined }}>
                        {p.status === "error" ? "Falhou" : timecode(p.duration, false)} · {dateTime(p.created_at)}
                      </span>
                    )}
                  </div>
                  <span className="btn btn-ghost btn-icon btn-sm btn-danger" role="button" aria-label="Apagar" onClick={(e) => { e.stopPropagation(); remove(p); }}><Trash2 {...ICON} /></span>
                </button>
              ))}
            </div>
          )}
        </div>
        {sel !== null ? <ProjectDetail key={sel} id={sel} onChanged={loadList} /> : <div className="card empty">Escolha um vídeo</div>}
      </section>
    </div>
  );
}

type Draft = { approved: boolean; layout: ClipLayout; camera: Rect; screen: Rect };

function ProjectDetail({ id, onChanged }: { id: number; onChanged: () => void }) {
  const toast = useToast();
  const [p, setP] = useState<ClipProjectView | null>(null);
  const [drafts, setDrafts] = useState<Record<number, Draft>>({});
  const [styles, setStyles] = useState<StyleRow[]>([]);
  const [styleId, setStyleId] = useState<number | 0>(0);
  const [marking, setMarking] = useState<Clip | null>(null);
  const [playing, setPlaying] = useState<number | null>(null);
  const video = useRef<HTMLVideoElement>(null);
  const stopAt = useRef<number | null>(null);

  const load = useCallback(() => api.get<ClipProjectView>(`/clips/${id}`).then((v) => {
    setP(v);
    setDrafts((cur) => {
      const next = { ...cur };
      for (const c of v.clips) if (!next[c.id]) next[c.id] = { approved: c.approved || c.score >= 70, layout: c.layout, camera: c.camera, screen: c.screen };
      return next;
    });
  }, (e) => toast("err", e.message)), [id, toast]);

  useEffect(() => { load(); api.get<StyleRow[]>("/caption-styles").then(setStyles, () => {}); }, [load]);

  const working = !!p && (busy(p) || p.clips.some((c) => c.status === "queued" || c.status === "processing"));
  useEffect(() => {
    if (!working) return;
    const t = setInterval(() => { load(); if (p && busy(p)) onChanged(); }, 2000);
    return () => clearInterval(t);
  }, [working, load]);

  if (!p) return <div className="card empty">Carregando</div>;

  if (busy(p) || p.status === "error") {
    return (
      <div className="card card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <span className="card-title">{p.title || p.url}</span>
        {p.status === "error" ? (
          <span style={{ color: "var(--err)" }}>{p.error_msg}</span>
        ) : (
          <>
            <div className="row" style={{ gap: 24 }}>
              {(["downloading", "transcribing", "analyzing"] as const).map((s, i) => {
                const order = ["downloading", "transcribing", "analyzing"].indexOf(p.status);
                const state = i < order ? "done" : i === order ? "now" : "next";
                return (
                  <span key={s} className="row" style={{ gap: 8, color: state === "next" ? "var(--text-3)" : "var(--text)" }}>
                    <span className={`status-dot ${state === "done" ? "ok" : ""}`} style={state === "now" ? { background: "var(--accent)" } : undefined} />
                    {STAGE[s]}{s === "downloading" && state === "now" ? <span className="mono muted"> {p.progress}%</span> : ""}
                  </span>
                );
              })}
            </div>
            <span className="muted" style={{ fontSize: 13 }}>Vídeos longos levam alguns minutos na transcrição local</span>
          </>
        )}
      </div>
    );
  }

  const src = apiUrl(`/clips/${p.id}/source`);
  const approved = p.clips.filter((c) => drafts[c.id]?.approved);
  const setD = (cid: number, d: Partial<Draft>) => setDrafts((cur) => ({ ...cur, [cid]: { ...cur[cid], ...d } }));

  function preview(c: Clip) {
    const v = video.current;
    if (!v) return;
    v.currentTime = c.start;
    stopAt.current = c.end;
    setPlaying(c.id);
    v.play();
  }

  async function render() {
    try {
      const v = await api.post<ClipProjectView>(`/clips/${p!.id}/render`, {
        caption_style_id: styleId || null,
        clips: p!.clips.map((c) => ({ id: c.id, ...drafts[c.id] })),
      });
      setP(v);
      toast("ok", `${approved.length} clipes na fila`);
    } catch (e: any) { toast("err", e.message); }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12, minWidth: 0 }}>
      <div className="card">
        <div className="card-head">
          <span className="card-title" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.title}</span>
          <span className="meta mono">{timecode(p.duration, false)} · {p.clips.length} sugestões</span>
        </div>
        <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          <video ref={video} src={src} controls preload="metadata"
            style={{ width: "100%", maxHeight: 360, background: "#000", borderRadius: "var(--r)" }}
            onTimeUpdate={(e) => { if (stopAt.current && e.currentTarget.currentTime >= stopAt.current) { e.currentTarget.pause(); stopAt.current = null; setPlaying(null); } }} />
          {/* Mapa de retenção: cada sugestão no tempo, mais opaca quanto maior a nota. */}
          <div style={{ position: "relative", height: 36, background: "var(--bg-deep)", border: "1px solid var(--line)", borderRadius: "var(--r)" }}>
            <div className="ruler-ticks" style={{ position: "absolute", inset: "0 0 auto 0", height: 10 }} />
            {p.clips.map((c) => (
              <button key={c.id} title={`${c.title} · ${c.score}`} onClick={() => preview(c)} style={{
                position: "absolute", top: 12, bottom: 4, left: `${(c.start / p.duration) * 100}%`, width: `${((c.end - c.start) / p.duration) * 100}%`,
                background: drafts[c.id]?.approved ? "var(--accent)" : "var(--raised-2)", opacity: 0.35 + (c.score / 100) * 0.65, borderRadius: 3,
              }} />
            ))}
          </div>
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <span className="card-title">Sugestões da IA</span>
          <span className="meta mono">{approved.length} aprovados</span>
        </div>
        <div className="list">
          {p.clips.map((c, i) => {
            const d = drafts[c.id];
            if (!d) return null;
            return (
              <div key={c.id} className="list-row" style={{ gridTemplateColumns: "28px 56px minmax(0, 1fr) 200px 140px", alignItems: "start", padding: "14px 20px", background: d.approved ? "var(--select)" : undefined }}>
                <button onClick={() => setD(c.id, { approved: !d.approved })} aria-label={d.approved ? "Desaprovar" : "Aprovar"} style={{
                  width: 20, height: 20, borderRadius: 6, marginTop: 2, display: "grid", placeItems: "center",
                  background: d.approved ? "var(--accent)" : "transparent", border: `1px solid ${d.approved ? "var(--accent)" : "var(--line-2)"}`,
                }}>{d.approved && <Check size={14} strokeWidth={2} />}</button>
                <div style={{ display: "flex", flexDirection: "column" }}>
                  <span className="mono" style={{ fontSize: 22, lineHeight: 1, color: i === 0 ? "var(--accent-ink)" : "var(--text)" }}>{c.score}</span>
                  <span className="muted" style={{ fontSize: 11 }}>retenção</span>
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
                  <span style={{ fontWeight: 500 }}>{c.title}</span>
                  <span className="dim" style={{ fontSize: 13 }}>{c.reason}</span>
                  <span className="mono muted" style={{ fontSize: 12 }}>{timecode(c.start)} → {timecode(c.end)} · {Math.round(c.end - c.start)}s</span>
                  {c.status !== "none" && (
                    <div className="row" style={{ gap: 8, marginTop: 4 }}>
                      {c.status === "done" && c.output_path ? (
                        <a className="btn btn-sm" href={apiUrl(`/output/${encodeURIComponent(basename(c.output_path))}?download=true`)}><Download {...ICON} />Baixar clipe</a>
                      ) : c.status === "error" ? (
                        <span style={{ color: "var(--err)", fontSize: 12 }}>{c.error_msg}</span>
                      ) : (
                        <div className="progress" style={{ width: 200 }}><i style={{ width: `${c.progress}%` }} /></div>
                      )}
                    </div>
                  )}
                </div>
                <select className="select" value={d.layout} onChange={(e) => { const l = e.target.value as ClipLayout; setD(c.id, { layout: l, ...suggestRects(l, p.width, p.height) }); }} style={{ height: 32, fontSize: 13 }}>
                  {LAYOUTS.map(([k, l]) => <option key={k} value={k}>{l}</option>)}
                </select>
                <div className="row" style={{ gap: 4, justifyContent: "flex-end" }}>
                  <button className="btn btn-ghost btn-icon btn-sm" onClick={() => preview(c)} aria-label="Assistir trecho" title="Assistir trecho">
                    <Play {...ICON} color={playing === c.id ? "var(--accent-ink)" : undefined} />
                  </button>
                  <button className="btn btn-sm" onClick={() => setMarking(c)} disabled={d.layout === "horizontal"} title="Marcar câmera e tela"><Crop {...ICON} />Áreas</button>
                </div>
              </div>
            );
          })}
        </div>
        <div className="card-pad row" style={{ borderTop: "1px solid var(--line)", justifyContent: "space-between", gap: 12 }}>
          <label className="row" style={{ gap: 8 }}>
            <span className="dim" style={{ fontSize: 13 }}>Legenda</span>
            <select className="select" value={styleId} onChange={(e) => setStyleId(Number(e.target.value))} style={{ height: 32, fontSize: 13 }}>
              <option value={0}>Sem legenda</option>
              {styles.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
            </select>
          </label>
          <button className="btn btn-primary" onClick={render} disabled={!approved.length}>
            <Scissors {...ICON} />Cortar {approved.length} {approved.length === 1 ? "clipe" : "clipes"}
          </button>
        </div>
      </div>

      {marking && drafts[marking.id] && (
        <RectEditor
          src={src}
          at={marking.start + 0.5}
          srcW={p.width}
          srcH={p.height}
          layout={drafts[marking.id].layout}
          camera={drafts[marking.id].camera}
          screen={drafts[marking.id].screen}
          onClose={() => setMarking(null)}
          onSave={(camera, screen) => { setD(marking.id, { camera, screen }); setMarking(null); }}
        />
      )}
    </div>
  );
}
