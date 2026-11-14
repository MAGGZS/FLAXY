import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { Captions } from "lucide-react";
import { api, type StyleRow } from "../lib/api";
import { CaptionPreview } from "./CaptionStyles";

const ICON = { size: 16, strokeWidth: 1.5 };

export type CaptionChoice = { on: boolean; styleId: number | null };

/** Liga a legenda do lote e escolhe o estilo. A transcrição é por vídeo. */
export function CaptionPanel({
  value, onChange, total, done, running, onTranscribe,
}: {
  value: CaptionChoice;
  onChange: (v: CaptionChoice) => void;
  total: number;
  done: number;
  running: number;
  onTranscribe: () => void;
}) {
  const [styles, setStyles] = useState<StyleRow[]>([]);
  useEffect(() => {
    api.get<StyleRow[]>("/caption-styles").then((r) => {
      setStyles(r);
      if (!value.styleId || !r.some((s) => s.id === value.styleId)) onChange({ ...value, styleId: r[0]?.id ?? null });
    }, () => {});
  }, []);

  const style = styles.find((s) => s.id === value.styleId);
  const missing = total - done - running;

  return (
    <div className="card">
      <div className="card-head">
        <Captions {...ICON} color="var(--text-3)" />
        <span className="card-title">Legenda automática</span>
        <div className="seg" style={{ marginLeft: "auto" }}>
          <button className={!value.on ? "on" : ""} onClick={() => onChange({ ...value, on: false })}>Não</button>
          <button className={value.on ? "on" : ""} onClick={() => onChange({ ...value, on: true })}>Sim</button>
        </div>
      </div>
      {value.on && (
        <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <label className="field">
            <span>Estilo</span>
            <select className="select" value={value.styleId ?? ""} onChange={(e) => onChange({ ...value, styleId: Number(e.target.value) })}>
              {styles.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
            </select>
          </label>
          {style && <CaptionPreview st={style.style} height={240} />}
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <div className="row" style={{ justifyContent: "space-between" }}>
              <span className="dim">Transcritos</span>
              <span className="mono muted" style={{ fontSize: 12 }}>{done}/{total}{running ? ` · ${running} em andamento` : ""}</span>
            </div>
            <div className="progress"><i style={{ width: `${total ? (done / total) * 100 : 0}%` }} /></div>
          </div>
          <button className="btn" onClick={onTranscribe} disabled={missing <= 0}>
            {missing > 0 ? `Transcrever ${missing} ${missing === 1 ? "vídeo" : "vídeos"}` : "Tudo transcrito"}
          </button>
          <Link to="/ia" className="muted" style={{ fontSize: 12 }}>Provedor e idioma na aba IA</Link>
        </div>
      )}
    </div>
  );
}
