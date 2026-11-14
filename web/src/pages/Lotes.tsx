import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Ban, Captions, Crop, RefreshCw, Trash2, Upload, Film, Play } from "lucide-react";
import { api, apiUrl, basename, uploadFiles, type Job, type Overrides, type Template, type Transcript, type Uploaded } from "../lib/api";
import { dateTime, ratioLabel, STATUS_LABEL, timecode } from "../lib/format";
import { useProgress } from "../lib/useProgress";
import { useToast } from "../components/Toast";
import { ClipEditor, FULL_FRAME, isAdjusted } from "../components/ClipEditor";
import { AudioPanel, audioOverrides, type AudioTrack } from "../components/AudioPanel";
import { CaptionPanel, type CaptionChoice } from "../components/CaptionPanel";
import { TranscriptEditor } from "../components/TranscriptEditor";

const ICON = { size: 16, strokeWidth: 1.5 };
const DRAFT_KEY = "flaxy.lote.rascunho";

type Tr = { id: number; status: Transcript["status"]; error?: string | null };
type Item = Uploaded & { overrides: Overrides; tr?: Tr };

type Draft = { items: Item[]; templateId: number | null; audio?: AudioTrack | null; captions?: CaptionChoice };

function loadDraft(): Draft {
  try {
    const raw = localStorage.getItem(DRAFT_KEY);
    if (raw) return JSON.parse(raw);
  } catch { /* sem storage */ }
  return { items: [], templateId: null };
}

export function Lotes() {
  const toast = useToast();
  const draft = useMemo(loadDraft, []);
  const [templates, setTemplates] = useState<Template[]>([]);
  const [templateId, setTemplateId] = useState<number | null>(draft.templateId);
  const [items, setItems] = useState<Item[]>(draft.items);
  const [audio, setAudio] = useState<AudioTrack | null>(draft.audio ?? null);
  const [captions, setCaptions] = useState<CaptionChoice>(draft.captions ?? { on: false, styleId: null });
  const [trEdit, setTrEdit] = useState<Item | null>(null);
  const [uploading, setUploading] = useState<number | null>(null);
  const [over, setOver] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [sending, setSending] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const template = templates.find((t) => t.id === templateId) ?? null;

  const loadJobs = useCallback(() => api.get<Job[]>("/jobs?limit=20").then(setJobs, () => {}), []);

  useEffect(() => {
    api.get<Template[]>("/templates").then((t) => {
      setTemplates(t);
      setTemplateId((cur) => (cur && t.some((x) => x.id === cur) ? cur : t[0]?.id ?? null));
    }, (e) => toast("err", e.message));
    loadJobs();
    // Remove do rascunho vídeos que sumiram do servidor (reinício, limpeza).
    api.get<{ items: { name: string }[] }>("/uploads").then((r) => {
      const alive = new Set(r.items.map((i) => i.name));
      setItems((cur) => cur.filter((i) => alive.has(i.file)));
      setAudio((cur) => (cur && !alive.has(cur.file) ? null : cur));
    }, () => {});
  }, [loadJobs, toast]);

  useEffect(() => {
    try { localStorage.setItem(DRAFT_KEY, JSON.stringify({ items, templateId, audio, captions })); } catch { /* sem storage */ }
  }, [items, templateId, audio, captions]);

  // Acompanha transcrições em andamento.
  const pending = items
    .filter((i) => i.tr && (i.tr.status === "queued" || i.tr.status === "processing"))
    .map((i) => i.tr!.id)
    .join(",");
  useEffect(() => {
    if (!pending) return;
    const t = setInterval(async () => {
      const ids = pending.split(",").map(Number);
      const res = await Promise.all(ids.map((id) => api.get<Transcript>(`/transcripts/${id}`).catch(() => null)));
      setItems((cur) => cur.map((it) => {
        const r = res.find((x) => x && it.tr && x.id === it.tr.id);
        return r ? { ...it, tr: { id: r.id, status: r.status, error: r.error_msg } } : it;
      }));
    }, 2000);
    return () => clearInterval(t);
  }, [pending]);

  async function transcribeAll() {
    for (const it of items.filter((i) => !i.tr || i.tr.status === "error")) {
      try {
        const t = await api.post<Transcript>("/transcripts", { path: it.path });
        setItems((cur) => cur.map((x) => (x.file === it.file ? { ...x, tr: { id: t.id, status: t.status } } : x)));
      } catch (e: any) {
        toast("err", e.message);
        break;
      }
    }
  }

  // Progresso ao vivo: atualiza o item no lugar, sem recarregar a lista.
  useProgress((e) => {
    setJobs((cur) => cur.map((j) => j.id !== e.job_id ? j : {
      ...j,
      items: j.items.map((i) => i.id === e.item_id ? { ...i, status: e.status, progress: e.progress } : i),
    }));
    if (e.status !== "processing") loadJobs();
  });

  async function addFiles(files: File[]) {
    const vids = files.filter((f) => f.type.startsWith("video/") || /\.(mp4|mov|webm|mkv|avi|m4v)$/i.test(f.name));
    if (!vids.length) return;
    setUploading(0);
    try {
      const r = await uploadFiles("/upload", vids, "files", setUploading);
      setItems((cur) => [...cur, ...r.items.map((u: Uploaded) => ({ ...u, overrides: { ...FULL_FRAME } }))]);
    } catch (e: any) {
      toast("err", e.message);
    } finally {
      setUploading(null);
    }
  }

  function remove(idx: number) {
    const it = items[idx];
    setItems((cur) => cur.filter((_, i) => i !== idx));
    api.del(`/uploads/${encodeURIComponent(it.file)}`).catch(() => {});
  }

  function clearAll() {
    setItems([]);
    api.post("/uploads/prune", { keep: audio ? [audio.file] : [] }).catch(() => {});
  }

  async function submit() {
    if (!template || !items.length) return;
    if (captions.on && items.some((i) => i.tr?.status !== "done")) {
      toast("err", "Transcreva todos os vídeos antes de renderizar com legenda");
      return;
    }
    setSending(true);
    try {
      await api.post<Job>("/jobs", {
        template_id: template.id,
        items: items.map((i) => ({
          path: i.path, ...i.overrides, ...audioOverrides(audio),
          ...(captions.on && captions.styleId && i.tr ? { caption_style_id: captions.styleId, transcript_id: i.tr.id } : {}),
        })),
      });
      toast("ok", `Lote com ${items.length} vídeos na fila`);
      setItems([]);
      loadJobs();
    } catch (e: any) {
      toast("err", e.message);
    } finally {
      setSending(false);
    }
  }

  const adjusted = items.filter((i) => isAdjusted(i.overrides)).length;
  const totalDur = items.reduce((s, i) => s + Math.max(0, (i.overrides.trim_end ?? i.duration) - (i.overrides.trim_start ?? 0)), 0);
  const editingItem = editing !== null ? items[editing] : null;

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">Lotes</h1>
          <p className="page-sub">Envie os vídeos, ajuste o que precisar e renderize todos no mesmo modelo</p>
        </div>
      </header>

      <section style={{ display: "grid", gridTemplateColumns: "1fr 320px", gap: 12, alignItems: "start" }}>
        <div className="card">
          <div className="card-head">
            <span className="card-title">Vídeos do lote</span>
            <span className="meta mono">{items.length} vídeos · {timecode(totalDur, false)}{adjusted ? ` · ${adjusted} ajustados` : ""}</span>
            {items.length > 0 && <button className="btn btn-ghost btn-sm" onClick={clearAll}>Limpar</button>}
          </div>
          <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <div
              className={`drop${over ? " over" : ""}`}
              onDragOver={(e) => { e.preventDefault(); setOver(true); }}
              onDragLeave={() => setOver(false)}
              onDrop={(e) => { e.preventDefault(); setOver(false); addFiles([...e.dataTransfer.files]); }}
            >
              <Upload size={20} strokeWidth={1.5} color="var(--text-3)" />
              <div style={{ flex: 1 }}>
                <div>{uploading !== null ? "Enviando" : "Arraste vídeos aqui"}</div>
                <div className="muted" style={{ fontSize: 13 }}>
                  {uploading !== null ? <span className="mono">{uploading}%</span> : "mp4, mov, webm, mkv, avi"}
                </div>
                {uploading !== null && <div className="progress" style={{ marginTop: 8 }}><i style={{ width: `${uploading}%` }} /></div>}
              </div>
              <button className="btn" onClick={() => input.current?.click()} disabled={uploading !== null}>Escolher arquivos</button>
              <input ref={input} type="file" accept="video/*" multiple hidden onChange={(e) => { addFiles([...(e.target.files ?? [])]); e.target.value = ""; }} />
            </div>

            {items.length > 0 && (
              <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(200px, 1fr))", gap: 12 }}>
                {items.map((it, idx) => {
                  const o = it.overrides;
                  const a = o.trim_start ?? 0, b = o.trim_end ?? it.duration;
                  return (
                    <div key={it.file} style={{ background: "var(--raised)", border: `1px solid ${isAdjusted(o) ? "var(--accent)" : "var(--line)"}`, borderRadius: "var(--r)", overflow: "hidden" }}>
                      <div style={{ position: "relative", aspectRatio: "16 / 9", background: "#000" }}>
                        <video src={`${apiUrl(`/uploads/${encodeURIComponent(it.file)}`)}#t=${a + 0.1}`} preload="metadata" muted style={{ width: "100%", height: "100%", objectFit: "cover" }} />
                        <span className="mono" style={{ position: "absolute", right: 6, bottom: 6, fontSize: 12, padding: "2px 6px", background: "rgba(5,6,8,0.8)", borderRadius: 4 }}>{timecode(b - a, false)}</span>
                      </div>
                      {it.duration > 0 && (
                        <div style={{ position: "relative", height: 3, background: "var(--bg-deep)" }}>
                          <i style={{ position: "absolute", top: 0, bottom: 0, left: `${(a / it.duration) * 100}%`, width: `${((b - a) / it.duration) * 100}%`, background: "var(--accent)" }} />
                        </div>
                      )}
                      <div style={{ padding: "8px 8px 8px 12px", display: "flex", alignItems: "center", gap: 4 }}>
                        <span title={it.name} style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontSize: 13 }}>{it.name}</span>
                        {it.tr && <TrBadge tr={it.tr} onEdit={() => setTrEdit(it)} />}
                        <button className="btn btn-ghost btn-icon btn-sm" onClick={() => setEditing(idx)} aria-label="Ajustar"><Crop {...ICON} /></button>
                        <button className="btn btn-ghost btn-icon btn-sm btn-danger" onClick={() => remove(idx)} aria-label="Remover"><Trash2 {...ICON} /></button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        <aside style={{ position: "sticky", top: 24, display: "flex", flexDirection: "column", gap: 12 }}>
        <div className="card">
          <div className="card-head"><span className="card-title">Modelo</span></div>
          <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            {templates.length === 0 ? (
              <div className="dim">Nenhum modelo de fundo. <Link to="/modelos" style={{ color: "var(--accent-ink)" }}>Criar modelo</Link></div>
            ) : (
              <>
                <label className="field">
                  <span>Fundo</span>
                  <select className="select" value={templateId ?? ""} onChange={(e) => setTemplateId(Number(e.target.value))}>
                    {templates.map((t) => <option key={t.id} value={t.id}>{t.name}</option>)}
                  </select>
                </label>
                {template && <TemplatePreview t={template} />}
              </>
            )}
            <button className="btn btn-primary" onClick={submit} disabled={!template || !items.length || sending || uploading !== null}>
              <Play {...ICON} />Renderizar {items.length || ""} {items.length === 1 ? "vídeo" : "vídeos"}
            </button>
          </div>
        </div>
        <AudioPanel value={audio} onChange={setAudio} />
        <CaptionPanel
          value={captions}
          onChange={setCaptions}
          total={items.length}
          done={items.filter((i) => i.tr?.status === "done").length}
          running={items.filter((i) => i.tr && (i.tr.status === "queued" || i.tr.status === "processing")).length}
          onTranscribe={transcribeAll}
        />
        </aside>
      </section>

      <section className="card">
        <div className="card-head">
          <span className="card-title">Fila e histórico</span>
          <span className="meta mono">{jobs.length} lotes</span>
        </div>
        {jobs.length === 0 ? (
          <div className="empty"><Film size={20} strokeWidth={1.5} />Nenhum lote enviado</div>
        ) : (
          <div className="list">{jobs.map((j) => <JobRow key={j.id} job={j} onChange={loadJobs} />)}</div>
        )}
      </section>

      {trEdit?.tr && <TranscriptEditor id={trEdit.tr.id} name={trEdit.name} onClose={() => setTrEdit(null)} />}

      {editingItem && (
        <ClipEditor
          src={apiUrl(`/uploads/${encodeURIComponent(editingItem.file)}`)}
          name={editingItem.name}
          duration={editingItem.duration}
          value={editingItem.overrides}
          template={template}
          onClose={() => setEditing(null)}
          onSave={(o) => {
            setItems((cur) => cur.map((it, i) => (i === editing ? { ...it, overrides: o } : it)));
            setEditing(null);
          }}
        />
      )}
    </div>
  );
}

function TrBadge({ tr, onEdit }: { tr: Tr; onEdit: () => void }) {
  if (tr.status === "done")
    return (
      <button className="btn btn-ghost btn-icon btn-sm" onClick={onEdit} title="Editar legenda" aria-label="Editar legenda">
        <Captions {...ICON} color="var(--accent-ink)" />
      </button>
    );
  if (tr.status === "error")
    return <span title={tr.error ?? ""} style={{ color: "var(--err)", display: "inline-flex" }}><Captions {...ICON} /></span>;
  return <span className="mono muted" style={{ fontSize: 11 }} title="Transcrevendo">{tr.status === "queued" ? "fila" : "transcrevendo"}</span>;
}

function TemplatePreview({ t }: { t: Template }) {
  const isImg = /\.(png|jpe?g|webp|bmp|gif)$/i.test(t.file_path);
  const src = apiUrl(`/templates/${t.id}/file`);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ position: "relative", aspectRatio: `${t.output_w} / ${t.output_h}`, maxHeight: 360, margin: "0 auto", background: "#000", borderRadius: "var(--r)", overflow: "hidden" }}>
        {isImg ? <img src={src} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
          : <video src={`${src}#t=0.1`} muted preload="metadata" style={{ width: "100%", height: "100%", objectFit: "cover" }} />}
        <div style={{
          position: "absolute", border: "1px solid var(--accent)", background: "rgba(59,99,214,0.18)",
          left: `${(t.overlay_x / t.output_w) * 100}%`, top: `${(t.overlay_y / t.output_h) * 100}%`,
          width: `${(t.overlay_w / t.output_w) * 100}%`, height: `${(t.overlay_h / t.output_h) * 100}%`,
        }} />
      </div>
      <span className="mono muted" style={{ fontSize: 12, textAlign: "center" }}>
        {t.output_w}×{t.output_h} · {ratioLabel(t.output_w, t.output_h)} · área {t.overlay_w}×{t.overlay_h}
      </span>
    </div>
  );
}

function JobRow({ job, onChange }: { job: Job; onChange: () => void }) {
  const toast = useToast();
  const [open, setOpen] = useState(job.status === "processing" || job.status === "queued");
  const done = job.items.filter((i) => i.status === "done").length;
  const pct = job.items.length ? job.items.reduce((s, i) => s + (i.status === "done" ? 100 : i.status === "processing" ? i.progress : 0), 0) / job.items.length : 0;
  const active = job.status === "processing" || job.status === "queued";

  const act = (p: Promise<unknown>) => p.then(onChange, (e) => toast("err", e.message));

  return (
    <>
      <div className="list-row" style={{ gridTemplateColumns: "80px 1fr 240px 72px 200px 40px", cursor: "pointer" }} onClick={() => setOpen((v) => !v)}>
        <span className="mono muted">#{String(job.id).padStart(4, "0")}</span>
        <span>{job.items.length} vídeos <span className="muted">· {STATUS_LABEL[job.status]}</span></span>
        <div className={`progress ${job.status === "done" ? "done" : job.status === "error" ? "err" : ""}`}><i style={{ width: `${pct}%` }} /></div>
        <span className="mono dim" style={{ textAlign: "right" }}>{done}/{job.items.length}</span>
        <span className="mono muted" style={{ textAlign: "right", fontSize: 12 }}>{dateTime(job.created_at)}</span>
        {active ? (
          <button className="btn btn-ghost btn-icon btn-sm" title="Cancelar fila" onClick={(e) => { e.stopPropagation(); act(api.post(`/jobs/${job.id}/cancel`)); }}><Ban {...ICON} /></button>
        ) : <span />}
      </div>
      {open && job.items.map((it) => (
        <div key={it.id} className="list-row" style={{ gridTemplateColumns: "80px 1fr 240px 72px 200px 40px", minHeight: 40, background: "var(--bg-deep)" }}>
          <span />
          <span className="dim" style={{ fontSize: 13, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={it.error_msg ?? ""}>
            {basename(it.input_path)}
            {it.error_msg && <span style={{ color: "var(--err)" }}> · {it.error_msg}</span>}
          </span>
          <div className={`progress ${it.status === "done" ? "done" : it.status === "error" ? "err" : ""}`}><i style={{ width: `${it.status === "done" ? 100 : it.progress}%` }} /></div>
          <span className="mono muted" style={{ textAlign: "right", fontSize: 12 }}>{it.status === "processing" ? `${it.progress}%` : ""}</span>
          <span className="muted" style={{ textAlign: "right", fontSize: 12 }}>
            {it.status === "done" && it.output_path
              ? <a href={apiUrl(`/output/${encodeURIComponent(basename(it.output_path))}?download=true`)} style={{ color: "var(--accent-ink)" }}>Baixar</a>
              : STATUS_LABEL[it.status]}
          </span>
          {(it.status === "error" || it.status === "canceled") ? (
            <button className="btn btn-ghost btn-icon btn-sm" title="Tentar de novo" onClick={() => act(api.post(`/jobs/${job.id}/items/${it.id}/retry`))}><RefreshCw {...ICON} /></button>
          ) : <span />}
        </div>
      ))}
    </>
  );
}
