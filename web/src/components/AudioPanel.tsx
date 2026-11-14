import { useRef, useState } from "react";
import { Music, Trash2, Upload } from "lucide-react";
import { apiUrl, uploadFiles } from "../lib/api";
import { timecode } from "../lib/format";
import { useToast } from "./Toast";

const ICON = { size: 16, strokeWidth: 1.5 };

/** Faixa extra aplicada a todos os vídeos do lote. */
export type AudioTrack = {
  path: string;
  file: string;
  name: string;
  duration: number;
  volume: number;
  original: number;
  offset: number;
  fade: boolean;
};

/** Campos que vão para os overrides de cada item no envio do lote. */
export const audioOverrides = (a: AudioTrack | null) =>
  a
    ? {
        audio_path: a.path,
        audio_volume: a.volume,
        original_volume: a.original,
        audio_offset: a.offset || null,
        audio_fade: a.fade,
      }
    : {};

export function AudioPanel({ value, onChange }: { value: AudioTrack | null; onChange: (a: AudioTrack | null) => void }) {
  const toast = useToast();
  const input = useRef<HTMLInputElement>(null);
  const [pct, setPct] = useState<number | null>(null);

  async function pick(f?: File) {
    if (!f) return;
    setPct(0);
    try {
      const r = await uploadFiles("/upload-audio", [f], "file", setPct);
      onChange({ ...r, volume: 0.6, original: 1, offset: 0, fade: true });
    } catch (e: any) {
      toast("err", e.message);
    } finally {
      setPct(null);
    }
  }

  const set = (p: Partial<AudioTrack>) => value && onChange({ ...value, ...p });

  return (
    <div className="card">
      <div className="card-head">
        <span className="card-title">Faixa de áudio</span>
        {value && <span className="meta mono">{timecode(value.duration, false)}</span>}
      </div>
      <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {!value ? (
          <>
            <span className="dim" style={{ fontSize: 13 }}>Música ou narração mixada em todos os vídeos. Repete se for mais curta</span>
            <button className="btn" onClick={() => input.current?.click()} disabled={pct !== null}>
              <Upload {...ICON} />{pct !== null ? <span className="mono">{pct}%</span> : "Escolher áudio"}
            </button>
          </>
        ) : (
          <>
            <div className="row" style={{ gap: 8 }}>
              <Music {...ICON} color="var(--accent-ink)" />
              <span title={value.name} style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{value.name}</span>
              <button className="btn btn-ghost btn-icon btn-sm btn-danger" onClick={() => onChange(null)} aria-label="Remover faixa"><Trash2 {...ICON} /></button>
            </div>
            <audio src={apiUrl(`/uploads/${encodeURIComponent(value.file)}`)} controls preload="none" style={{ width: "100%", height: 32 }} />

            <label className="field">
              <span>Volume da faixa <span className="mono">{Math.round(value.volume * 100)}%</span></span>
              <input type="range" min={0} max={2} step={0.05} value={value.volume} onChange={(e) => set({ volume: Number(e.target.value) })} />
            </label>
            <label className="field">
              <span>Áudio original <span className="mono">{value.original === 0 ? "mudo" : `${Math.round(value.original * 100)}%`}</span></span>
              <input type="range" min={0} max={2} step={0.05} value={value.original} onChange={(e) => set({ original: Number(e.target.value) })} />
            </label>
            <div className="grid-2">
              <label className="field">
                <span>Começar a faixa em (s)</span>
                <input className="input mono" inputMode="decimal" value={value.offset} onChange={(e) => set({ offset: Math.max(0, Number(e.target.value.replace(",", ".")) || 0) })} />
              </label>
              <div className="field">
                <span>Final</span>
                <div className="seg">
                  <button className={value.fade ? "on" : ""} onClick={() => set({ fade: true })}>Fade</button>
                  <button className={!value.fade ? "on" : ""} onClick={() => set({ fade: false })}>Corte seco</button>
                </div>
              </div>
            </div>
          </>
        )}
        <input ref={input} type="file" accept="audio/*,.mp3,.wav,.m4a,.aac,.ogg,.opus,.flac" hidden onChange={(e) => { pick(e.target.files?.[0]); e.target.value = ""; }} />
      </div>
    </div>
  );
}
