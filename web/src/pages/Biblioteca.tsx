import { useCallback, useEffect, useState } from "react";
import { Archive, Check, Download, FolderOpen, Trash2, X } from "lucide-react";
import { api, apiUrl, type OutputFile } from "../lib/api";
import { bytes, dateTime } from "../lib/format";
import { useToast } from "../components/Toast";
import { useProgress } from "../lib/useProgress";

const ICON = { size: 16, strokeWidth: 1.5 };

export function Biblioteca() {
  const toast = useToast();
  const [files, setFiles] = useState<OutputFile[]>([]);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [playing, setPlaying] = useState<string | null>(null);

  const load = useCallback(() => api.get<{ items: OutputFile[] }>("/output").then((r) => setFiles(r.items), (e) => toast("err", e.message)), [toast]);
  useEffect(() => { load(); }, [load]);
  useProgress((e) => { if (e.status === "done") load(); });

  const toggle = (n: string) => setSel((s) => { const c = new Set(s); c.has(n) ? c.delete(n) : c.add(n); return c; });

  async function remove(names: string[]) {
    for (const n of names) await api.del(`/output/${encodeURIComponent(n)}`).catch(() => {});
    setSel(new Set());
    load();
  }

  const zipHref = apiUrl(`/output-zip?filenames=${encodeURIComponent([...sel].join(","))}`);
  const total = files.reduce((s, f) => s + f.size_bytes, 0);

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">Biblioteca</h1>
          <p className="page-sub"><span className="mono">{files.length}</span> vídeos prontos · <span className="mono">{bytes(total)}</span></p>
        </div>
        <div className="page-actions">
          {sel.size > 0 && (
            <>
              <button className="btn btn-ghost" onClick={() => setSel(new Set())}><X {...ICON} />Limpar seleção</button>
              <button className="btn btn-danger" onClick={() => remove([...sel])}><Trash2 {...ICON} />Apagar {sel.size}</button>
              <a className="btn btn-primary" href={zipHref}><Archive {...ICON} />Baixar {sel.size} em ZIP</a>
            </>
          )}
          {sel.size === 0 && files.length > 0 && (
            <button className="btn" onClick={() => setSel(new Set(files.map((f) => f.name)))}>Selecionar todos</button>
          )}
        </div>
      </header>

      {files.length === 0 ? (
        <div className="card empty"><FolderOpen size={20} strokeWidth={1.5} />Nada renderizado ainda</div>
      ) : (
        <section style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(184px, 1fr))", gap: 12 }}>
          {files.map((f) => {
            const on = sel.has(f.name);
            const src = apiUrl(`/output/${encodeURIComponent(f.name)}`);
            return (
              <article key={f.name} className="card" style={{ overflow: "hidden", borderColor: on ? "var(--accent)" : undefined, background: on ? "var(--select)" : undefined }}>
                <div style={{ position: "relative", aspectRatio: "9 / 16", background: "#000" }}>
                  {playing === f.name
                    ? <video src={src} controls autoPlay style={{ width: "100%", height: "100%", objectFit: "contain" }} />
                    : <video src={`${src}#t=0.5`} muted preload="metadata" onClick={() => setPlaying(f.name)} style={{ width: "100%", height: "100%", objectFit: "cover", cursor: "pointer" }} />}
                  <button onClick={() => toggle(f.name)} aria-label={on ? "Desmarcar" : "Selecionar"} style={{
                    position: "absolute", top: 8, left: 8, width: 24, height: 24, borderRadius: 6, display: "grid", placeItems: "center",
                    background: on ? "var(--accent)" : "rgba(5,6,8,0.7)", border: "1px solid var(--line-2)",
                  }}>{on && <Check size={14} strokeWidth={2} />}</button>
                </div>
                <div style={{ padding: "8px 8px 8px 12px", display: "flex", alignItems: "center", gap: 4 }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div title={f.name} style={{ fontSize: 13, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{f.name}</div>
                    <div className="mono muted" style={{ fontSize: 12 }}>{bytes(f.size_bytes)} · {dateTime(f.modified_at)}</div>
                  </div>
                  <a className="btn btn-ghost btn-icon btn-sm" href={`${src}${src.includes("?") ? "&" : "?"}download=true`} aria-label="Baixar"><Download {...ICON} /></a>
                </div>
              </article>
            );
          })}
        </section>
      )}
    </div>
  );
}
