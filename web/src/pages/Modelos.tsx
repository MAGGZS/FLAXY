import { useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { useSearchParams } from "react-router-dom";
import { Copy, ImagePlus, LayoutTemplate, Plus, Save, Trash2 } from "lucide-react";
import { api, apiUrl, uploadFiles, type Template, type TemplateInput } from "../lib/api";
import { ratioLabel } from "../lib/format";
import { useToast } from "../components/Toast";
import { CaptionStyles } from "../components/CaptionStyles";

const ICON = { size: 16, strokeWidth: 1.5 };

const PRESETS: [string, number, number][] = [
  ["9:16", 1080, 1920],
  ["4:5", 1080, 1350],
  ["1:1", 1080, 1080],
  ["16:9", 1920, 1080],
];

const BLANK: TemplateInput = {
  name: "", overlay_x: 0, overlay_y: 420, overlay_w: 1080, overlay_h: 1080, fit_mode: "cover",
  output_w: 1080, output_h: 1920, output_format: "mp4", video_bitrate: "8M",
  audio_source: "raw", audio_mix_raw: 1, audio_mix_template: 0.5, duration_rule: "raw",
};

type Draft = TemplateInput & { id?: number; file?: File; preview?: string };

const isImage = (p: string) => /\.(png|jpe?g|webp|bmp|gif)$/i.test(p);

export function Modelos() {
  const toast = useToast();
  const [list, setList] = useState<Template[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [saving, setSaving] = useState<number | null>(null);
  const [params, setParams] = useSearchParams();
  const tab = params.get("aba") === "legendas" ? "legendas" : "fundos";
  const setTab = (t: "fundos" | "legendas") => setParams(t === "fundos" ? {} : { aba: t }, { replace: true });

  const load = () => api.get<Template[]>("/templates").then(setList, (e) => toast("err", e.message));
  useEffect(() => { load(); }, []);

  function edit(t: Template) {
    const { id, file_path, created_at: _c, ...rest } = t;
    setDraft({ ...rest, id, preview: apiUrl(`/templates/${id}/file`) + (isImage(file_path) ? "" : "#t=0.1") });
  }

  async function save() {
    if (!draft) return;
    const { id, file, preview: _p, ...data } = draft;
    if (!data.name.trim()) return toast("err", "Dê um nome ao modelo");
    try {
      if (id) {
        await api.put(`/templates/${id}`, data);
      } else {
        if (!file) return toast("err", "Escolha o fundo do modelo");
        setSaving(0);
        await uploadFiles("/templates", [file], "file", setSaving, { data: JSON.stringify(data) });
      }
      toast("ok", "Modelo salvo");
      setDraft(null);
      load();
    } catch (e: any) {
      toast("err", e.message);
    } finally {
      setSaving(null);
    }
  }

  async function remove(t: Template) {
    if (!confirm(`Apagar o modelo "${t.name}"?`)) return;
    await api.del(`/templates/${t.id}`).catch((e) => toast("err", e.message));
    if (draft?.id === t.id) setDraft(null);
    load();
  }

  async function duplicate(t: Template) {
    await api.post(`/templates/${t.id}/duplicate`).catch((e) => toast("err", e.message));
    load();
  }

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">Modelos</h1>
          <p className="page-sub">{tab === "fundos" ? "Fundo fixo e a área onde cada vídeo entra" : "Como a legenda aparece no vídeo"}</p>
        </div>
        <div className="page-actions">
          <div className="seg" role="tablist" aria-label="Tipo de modelo">
            <button className={tab === "fundos" ? "on" : ""} role="tab" aria-selected={tab === "fundos"} onClick={() => setTab("fundos")}>Fundos</button>
            <button className={tab === "legendas" ? "on" : ""} role="tab" aria-selected={tab === "legendas"} onClick={() => setTab("legendas")}>Legendas</button>
          </div>
          {tab === "fundos" && <button className="btn btn-primary" onClick={() => setDraft({ ...BLANK })}><Plus {...ICON} />Novo modelo</button>}
        </div>
      </header>

      {tab === "legendas" ? <CaptionStyles /> : <section style={{ display: "grid", gridTemplateColumns: draft ? "minmax(0, 1fr) 640px" : "1fr", gap: 12, alignItems: "start" }}>
        {list.length === 0 && !draft ? (
          <div className="card empty"><LayoutTemplate size={20} strokeWidth={1.5} />Nenhum modelo de fundo</div>
        ) : (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(168px, 1fr))", gap: 12, alignContent: "start" }}>
            {list.map((t) => {
              const src = apiUrl(`/templates/${t.id}/file`);
              const on = draft?.id === t.id;
              return (
                <article key={t.id} className="card" style={{ overflow: "hidden", borderColor: on ? "var(--accent)" : undefined, cursor: "pointer" }} onClick={() => edit(t)}>
                  <div style={{ position: "relative", aspectRatio: "9 / 16", background: "#000", display: "grid", placeItems: "center" }}>
                    <div style={{ position: "relative", aspectRatio: `${t.output_w} / ${t.output_h}`, width: t.output_w >= t.output_h ? "100%" : undefined, height: t.output_w < t.output_h ? "100%" : undefined }}>
                      {isImage(t.file_path)
                        ? <img src={src} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
                        : <video src={`${src}#t=0.1`} muted preload="metadata" style={{ width: "100%", height: "100%", objectFit: "cover" }} />}
                      <div style={{ position: "absolute", border: "1px solid var(--accent)", background: "rgba(59,99,214,0.16)",
                        left: `${(t.overlay_x / t.output_w) * 100}%`, top: `${(t.overlay_y / t.output_h) * 100}%`,
                        width: `${(t.overlay_w / t.output_w) * 100}%`, height: `${(t.overlay_h / t.output_h) * 100}%` }} />
                    </div>
                  </div>
                  <div style={{ padding: "8px 8px 8px 12px", display: "flex", alignItems: "center", gap: 4 }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 13, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{t.name}</div>
                      <div className="mono muted" style={{ fontSize: 12 }}>{ratioLabel(t.output_w, t.output_h)} · {t.output_w}×{t.output_h}</div>
                    </div>
                    <button className="btn btn-ghost btn-icon btn-sm" onClick={(e) => { e.stopPropagation(); duplicate(t); }} aria-label="Duplicar"><Copy {...ICON} /></button>
                    <button className="btn btn-ghost btn-icon btn-sm btn-danger" onClick={(e) => { e.stopPropagation(); remove(t); }} aria-label="Apagar"><Trash2 {...ICON} /></button>
                  </div>
                </article>
              );
            })}
          </div>
        )}

        {draft && <Editor draft={draft} setDraft={setDraft} onSave={save} saving={saving} onCancel={() => setDraft(null)} />}
      </section>}
    </div>
  );
}

function Editor({ draft, setDraft, onSave, onCancel, saving }: {
  draft: Draft;
  setDraft: (d: Draft) => void;
  onSave: () => void;
  onCancel: () => void;
  saving: number | null;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const d = draft;
  const set = (p: Partial<Draft>) => setDraft({ ...d, ...p });

  function pickFile(f?: File) {
    if (!f) return;
    if (d.preview?.startsWith("blob:")) URL.revokeObjectURL(d.preview);
    set({ file: f, preview: URL.createObjectURL(f), name: d.name || f.name.replace(/\.[^.]+$/, "") });
  }

  function preset(w: number, h: number) {
    // Mantém a área proporcional ao trocar a saída.
    const sx = w / d.output_w, sy = h / d.output_h;
    set({
      output_w: w, output_h: h,
      overlay_x: Math.round(d.overlay_x * sx), overlay_y: Math.round(d.overlay_y * sy),
      overlay_w: Math.round(d.overlay_w * sx), overlay_h: Math.round(d.overlay_h * sy),
    });
  }

  function drag(mode: "move" | "se" | "nw", e: RPointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    const rect = stage.current!.getBoundingClientRect();
    const k = d.output_w / rect.width;
    const s = { x: e.clientX, y: e.clientY, ...d };
    const mv = (ev: PointerEvent) => {
      const dx = Math.round((ev.clientX - s.x) * k), dy = Math.round((ev.clientY - s.y) * k);
      let { overlay_x: x, overlay_y: y, overlay_w: w, overlay_h: h } = s;
      if (mode === "move") { x = Math.min(Math.max(0, x + dx), d.output_w - w); y = Math.min(Math.max(0, y + dy), d.output_h - h); }
      if (mode === "se") { w = Math.min(Math.max(32, w + dx), d.output_w - x); h = Math.min(Math.max(32, h + dy), d.output_h - y); }
      if (mode === "nw") {
        const nx = Math.min(Math.max(0, x + dx), x + w - 32), ny = Math.min(Math.max(0, y + dy), y + h - 32);
        w += x - nx; h += y - ny; x = nx; y = ny;
      }
      setDraft({ ...s, overlay_x: x, overlay_y: y, overlay_w: w & ~1, overlay_h: h & ~1 });
    };
    const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  }

  const isVid = d.file ? d.file.type.startsWith("video/") : d.preview?.includes("#t=");
  const num = (k: keyof TemplateInput) => (e: React.ChangeEvent<HTMLInputElement>) => set({ [k]: Number(e.target.value) || 0 } as Partial<Draft>);

  return (
    <aside className="card" style={{ position: "sticky", top: 24 }}>
      <div className="card-head">
        <span className="card-title">{d.id ? "Editar modelo" : "Novo modelo"}</span>
        <span className="meta mono">{d.output_w}×{d.output_h}</span>
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "240px 1fr" }}>
        <div style={{ padding: 20, borderRight: "1px solid var(--line)", display: "flex", flexDirection: "column", gap: 12 }}>
          <div ref={stage} style={{ position: "relative", aspectRatio: `${d.output_w} / ${d.output_h}`, background: "var(--bg-deep)", borderRadius: "var(--r)", overflow: "hidden", border: "1px solid var(--line)" }}>
            {d.preview ? (isVid
              ? <video src={d.preview} muted preload="metadata" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
              : <img src={d.preview} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} />)
              : <button onClick={() => fileInput.current?.click()} style={{ position: "absolute", inset: 0, display: "grid", placeItems: "center", color: "var(--text-3)" }}><ImagePlus size={20} strokeWidth={1.5} /></button>}
            <div onPointerDown={(e) => drag("move", e)} style={{
              position: "absolute", cursor: "move", outline: "1px solid var(--accent)", background: "rgba(59,99,214,0.22)",
              left: `${(d.overlay_x / d.output_w) * 100}%`, top: `${(d.overlay_y / d.output_h) * 100}%`,
              width: `${(d.overlay_w / d.output_w) * 100}%`, height: `${(d.overlay_h / d.output_h) * 100}%`,
            }}>
              <span className="mono" style={{ position: "absolute", left: 4, top: 4, fontSize: 11, color: "var(--text)" }}>vídeo</span>
              {(["nw", "se"] as const).map((m) => (
                <span key={m} onPointerDown={(e) => drag(m, e)} style={{ position: "absolute", width: 10, height: 10, background: "var(--text)", borderRadius: 2, cursor: `${m}-resize`, [m === "nw" ? "left" : "right"]: -5, [m === "nw" ? "top" : "bottom"]: -5 }} />
              ))}
            </div>
          </div>
          {!d.id && <button className="btn btn-sm" onClick={() => fileInput.current?.click()}><ImagePlus {...ICON} />{d.file ? "Trocar fundo" : "Escolher fundo"}</button>}
          <input ref={fileInput} type="file" accept="image/*,video/*" hidden onChange={(e) => pickFile(e.target.files?.[0])} />
        </div>

        <div style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
          <label className="field"><span>Nome</span><input className="input" value={d.name} onChange={(e) => set({ name: e.target.value })} /></label>

          <div className="field">
            <span>Proporção da saída</span>
            <div className="seg">
              {PRESETS.map(([l, w, h]) => <button key={l} className={d.output_w === w && d.output_h === h ? "on" : ""} onClick={() => preset(w, h)}>{l}</button>)}
            </div>
          </div>

          <div className="field">
            <span>Área do vídeo (px)</span>
            <div className="grid-2" style={{ gridTemplateColumns: "repeat(4, 1fr)", gap: 8 }}>
              {(["overlay_x", "overlay_y", "overlay_w", "overlay_h"] as const).map((k) => (
                <input key={k} className="input mono" aria-label={k} value={d[k]} onChange={num(k)} />
              ))}
            </div>
            <span className="mono muted" style={{ fontSize: 12 }}>x · y · largura · altura</span>
          </div>

          <div className="field">
            <span>Encaixe</span>
            <div className="seg">
              <button className={d.fit_mode === "cover" ? "on" : ""} onClick={() => set({ fit_mode: "cover" })}>Preencher (corta)</button>
              <button className={d.fit_mode === "contain" ? "on" : ""} onClick={() => set({ fit_mode: "contain" })}>Caber (bordas)</button>
            </div>
          </div>

          <div className="grid-2">
            <label className="field"><span>Duração</span>
              <select className="select" value={d.duration_rule} onChange={(e) => set({ duration_rule: e.target.value as Draft["duration_rule"] })}>
                <option value="raw">Do vídeo</option>
                <option value="template">Do fundo</option>
                <option value="loop_template">Fundo em loop</option>
              </select>
            </label>
            <label className="field"><span>Áudio</span>
              <select className="select" value={d.audio_source} onChange={(e) => set({ audio_source: e.target.value as Draft["audio_source"] })}>
                <option value="raw">Do vídeo</option>
                <option value="template">Do fundo</option>
                <option value="both">Mixar os dois</option>
              </select>
            </label>
          </div>

          {d.audio_source === "both" && (
            <div className="grid-2">
              <label className="field"><span>Volume do vídeo <span className="mono">{Math.round(d.audio_mix_raw * 100)}%</span></span>
                <input type="range" min={0} max={2} step={0.05} value={d.audio_mix_raw} onChange={(e) => set({ audio_mix_raw: Number(e.target.value) })} /></label>
              <label className="field"><span>Volume do fundo <span className="mono">{Math.round(d.audio_mix_template * 100)}%</span></span>
                <input type="range" min={0} max={2} step={0.05} value={d.audio_mix_template} onChange={(e) => set({ audio_mix_template: Number(e.target.value) })} /></label>
            </div>
          )}

          <label className="field"><span>Bitrate de vídeo</span>
            <select className="select" value={d.video_bitrate} onChange={(e) => set({ video_bitrate: e.target.value })}>
              {["4M", "6M", "8M", "12M", "16M"].map((b) => <option key={b} value={b}>{b}</option>)}
            </select>
          </label>

          <div className="row" style={{ justifyContent: "flex-end", gap: 8, marginTop: 8 }}>
            <button className="btn btn-ghost" onClick={onCancel}>Cancelar</button>
            <button className="btn btn-primary" onClick={onSave} disabled={saving !== null}>
              <Save {...ICON} />{saving !== null ? <span className="mono">{saving}%</span> : "Salvar modelo"}
            </button>
          </div>
        </div>
      </div>
    </aside>
  );
}
