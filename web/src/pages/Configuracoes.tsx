import { useEffect, useState } from "react";
import { api, type Health } from "../lib/api";

export function Configuracoes() {
  const [h, setH] = useState<Health | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => { api.get<Health>("/health").then(setH, (e) => setErr(e.message)); }, []);

  const rows: [string, string, string][] = h ? [
    ["Versão do servidor", `v${h.version}`, "Núcleo Rust"],
    ["FFmpeg", h.ffmpeg ? "Encontrado" : "Ausente", h.ffmpeg ? "No PATH ou em FLAXY_FFMPEG_DIR" : "Instale e reinicie o servidor"],
    ["Encoder de vídeo", h.encoder, h.encoder === "libx264" ? "CPU. Nenhuma GPU compatível respondeu ao teste" : "GPU detectada no teste de encode"],
    ["Vídeos por lote", String(h.max_videos_per_job), "FLAXY_MAX_VIDEOS_PER_JOB"],
    ["Limite por arquivo", `${h.max_upload_mb.toLocaleString("pt-BR")} MB`, "FLAXY_MAX_UPLOAD_MB"],
  ] : [];

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">Configurações</h1>
          <p className="page-sub">Estado do servidor e limites. Valores vêm das variáveis de ambiente</p>
        </div>
      </header>
      <section className="card" style={{ maxWidth: 880 }}>
        {err && <div className="empty">{err}</div>}
        <div className="list">
          {rows.map(([k, v, note]) => (
            <div key={k} className="list-row" style={{ gridTemplateColumns: "220px 200px 1fr" }}>
              <span className="dim">{k}</span>
              <span className="mono">{v}</span>
              <span className="muted" style={{ fontSize: 13 }}>{note}</span>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}
