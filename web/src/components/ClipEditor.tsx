import { useEffect, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { Lock, LockOpen, Pause, Play, RotateCcw, X } from "lucide-react";
import type { Overrides, Template } from "../lib/api";
import { timecode } from "../lib/format";

const ICON = { size: 16, strokeWidth: 1.5 };
const FULL: Overrides = { crop_x: 0, crop_y: 0, crop_w: 1, crop_h: 1, fit_mode: null, trim_start: null, trim_end: null };

type Handle = "move" | "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

/**
 * Editor de um vídeo: recorte do quadro (fração 0..1 da fonte) e corte no
 * tempo (entrada/saída). O recorte é gravado em fração para valer igual em
 * qualquer resolução.
 */
export function ClipEditor({
  src, name, duration, value, template, onSave, onClose,
}: {
  src: string;
  name: string;
  duration: number;
  value: Overrides;
  template: Template | null;
  onSave: (o: Overrides) => void;
  onClose: () => void;
}) {
  const [o, setO] = useState<Overrides>({ ...FULL, ...value });
  const [lock, setLock] = useState(true);
  const [time, setTime] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [dims, setDims] = useState({ w: 16, h: 9 });
  const video = useRef<HTMLVideoElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const track = useRef<HTMLDivElement>(null);

  const dur = duration || video.current?.duration || 0;
  const tin = o.trim_start ?? 0;
  const tout = o.trim_end ?? dur;
  // Proporção da área do modelo, em fração da fonte.
  const targetRatio = template ? template.overlay_w / template.overlay_h : null;
  const fracRatio = targetRatio ? targetRatio / (dims.w / dims.h) : null;

  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === " " && e.target === document.body) { e.preventDefault(); toggle(); }
      if (e.key.toLowerCase() === "i") setIn(video.current?.currentTime ?? 0);
      if (e.key.toLowerCase() === "o") setOut(video.current?.currentTime ?? dur);
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  });

  // Mantém a reprodução dentro do trecho escolhido.
  const onTime = () => {
    const v = video.current!;
    setTime(v.currentTime);
    if (v.currentTime >= tout - 0.02 && !v.paused) { v.pause(); v.currentTime = tin; }
  };

  function toggle() {
    const v = video.current;
    if (!v) return;
    if (v.paused) { if (v.currentTime < tin || v.currentTime >= tout) v.currentTime = tin; v.play(); }
    else v.pause();
  }

  const setIn = (t: number) => setO((p) => ({ ...p, trim_start: clamp(t, 0, (p.trim_end ?? dur) - 0.1) || null }));
  const setOut = (t: number) => setO((p) => {
    const v = clamp(t, (p.trim_start ?? 0) + 0.1, dur);
    return { ...p, trim_end: v >= dur - 0.01 ? null : v };
  });

  // ------------------------------------------------------------- crop drag
  function startCrop(h: Handle, e: RPointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    const rect = frame.current!.getBoundingClientRect();
    const start = { x: e.clientX, y: e.clientY, ...o };
    const move = (ev: PointerEvent) => {
      const dx = (ev.clientX - start.x) / rect.width;
      const dy = (ev.clientY - start.y) / rect.height;
      let { crop_x: x, crop_y: y, crop_w: w, crop_h: hh } = start;
      if (h === "move") {
        x = clamp(x + dx, 0, 1 - w);
        y = clamp(y + dy, 0, 1 - hh);
      } else {
        if (h.includes("w")) { const nx = clamp(x + dx, 0, x + w - 0.05); w += x - nx; x = nx; }
        if (h.includes("e")) w = clamp(w + dx, 0.05, 1 - x);
        if (h.includes("n")) { const ny = clamp(y + dy, 0, y + hh - 0.05); hh += y - ny; y = ny; }
        if (h.includes("s")) hh = clamp(hh + dy, 0.05, 1 - y);
        if (lock && fracRatio) {
          // Lado arrastado manda; o outro segue a proporção do modelo.
          if (h === "n" || h === "s") w = hh * fracRatio; else hh = w / fracRatio;
          if (w > 1 - x) { w = 1 - x; hh = w / fracRatio; }
          if (hh > 1 - y) { hh = 1 - y; w = hh * fracRatio; }
        }
      }
      setO((p) => ({ ...p, crop_x: x, crop_y: y, crop_w: w, crop_h: hh }));
    };
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function fitTemplate() {
    if (!fracRatio) return;
    let w = 1, h = 1 / fracRatio;
    if (h > 1) { h = 1; w = fracRatio; }
    setO((p) => ({ ...p, crop_x: (1 - w) / 2, crop_y: (1 - h) / 2, crop_w: w, crop_h: h }));
  }

  // ------------------------------------------------------------- trim drag
  function startTrim(which: "in" | "out" | "seek", e: RPointerEvent) {
    e.preventDefault();
    const rect = track.current!.getBoundingClientRect();
    const at = (x: number) => clamp((x - rect.left) / rect.width, 0, 1) * dur;
    const apply = (x: number) => {
      const t = at(x);
      if (which === "in") setIn(t);
      else if (which === "out") setOut(t);
      if (video.current) video.current.currentTime = t;
    };
    apply(e.clientX);
    const move = (ev: PointerEvent) => apply(ev.clientX);
    const up = () => { window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  const pct = (t: number) => `${dur ? (t / dur) * 100 : 0}%`;
  const box = { left: `${o.crop_x * 100}%`, top: `${o.crop_y * 100}%`, width: `${o.crop_w * 100}%`, height: `${o.crop_h * 100}%` };
  const handles: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

  return (
    <div className="overlay" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-label={`Ajustar ${name}`}>
        <div className="card-head">
          <span className="card-title" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{name}</span>
          <span className="meta mono">{timecode(tout - tin)} de {timecode(dur)}</span>
          <button className="btn btn-ghost btn-icon btn-sm" onClick={onClose} aria-label="Fechar"><X {...ICON} /></button>
        </div>

        <div style={{ display: "grid", gridTemplateColumns: "1fr 280px", minHeight: 0 }}>
          <div style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16, borderRight: "1px solid var(--line)" }}>
            <div style={{ background: "#000", borderRadius: "var(--r)", display: "grid", placeItems: "center", height: "min(56vh, 560px)", overflow: "hidden" }}>
              <div ref={frame} style={{ position: "relative", aspectRatio: `${dims.w} / ${dims.h}`, maxHeight: "100%", maxWidth: "100%", height: "100%" }}>
                <video
                  ref={video} src={src} preload="auto" playsInline
                  style={{ width: "100%", height: "100%", objectFit: "contain" }}
                  onLoadedMetadata={(e) => setDims({ w: e.currentTarget.videoWidth || 16, h: e.currentTarget.videoHeight || 9 })}
                  onTimeUpdate={onTime}
                  onPlay={() => setPlaying(true)}
                  onPause={() => setPlaying(false)}
                />
                <div style={{ position: "absolute", ...box, boxShadow: "0 0 0 9999px rgba(5,6,8,0.66)", outline: "1px solid var(--text)", cursor: "move" }}
                  onPointerDown={(e) => startCrop("move", e)}>
                  <div style={{ position: "absolute", inset: 0, backgroundImage: "linear-gradient(to right, transparent 33.2%, var(--line-2) 33.3%, transparent 33.4%, transparent 66.5%, var(--line-2) 66.6%, transparent 66.7%), linear-gradient(transparent 33.2%, var(--line-2) 33.3%, transparent 33.4%, transparent 66.5%, var(--line-2) 66.6%, transparent 66.7%)" }} />
                  {handles.map((h) => (
                    <span key={h} onPointerDown={(e) => startCrop(h, e)} style={{
                      position: "absolute", width: 12, height: 12, background: "var(--text)", borderRadius: 2,
                      left: h.includes("w") ? -6 : h.includes("e") ? "calc(100% - 6px)" : "calc(50% - 6px)",
                      top: h.includes("n") ? -6 : h.includes("s") ? "calc(100% - 6px)" : "calc(50% - 6px)",
                      cursor: `${h}-resize`,
                    }} />
                  ))}
                </div>
              </div>
            </div>

            {/* Timeline: régua com ticks, trecho mantido em azul escuro, entrada e saída arrastáveis. */}
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <div className="row" style={{ justifyContent: "space-between" }}>
                <button className="btn btn-sm" onClick={toggle} aria-label={playing ? "Pausar" : "Reproduzir"}>
                  {playing ? <Pause {...ICON} /> : <Play {...ICON} />}
                  <span className="mono">{timecode(time)}</span>
                </button>
                <span className="mono muted" style={{ fontSize: 12 }}>entrada {timecode(tin)} · saída {timecode(tout)}</span>
              </div>
              <div ref={track} onPointerDown={(e) => startTrim("seek", e)} style={{ position: "relative", height: 48, background: "var(--bg-deep)", borderRadius: "var(--r)", border: "1px solid var(--line)", cursor: "text", touchAction: "none" }}>
                <div className="ruler-ticks" style={{ position: "absolute", inset: "0 0 auto 0", height: 12 }} />
                <div style={{ position: "absolute", top: 14, bottom: 6, left: pct(tin), width: `calc(${pct(tout)} - ${pct(tin)})`, background: "var(--select)", border: "1px solid var(--accent)", borderRadius: 4 }} />
                {(["in", "out"] as const).map((w) => (
                  <span key={w} onPointerDown={(e) => { e.stopPropagation(); startTrim(w, e); }} title={w === "in" ? "Entrada (I)" : "Saída (O)"} style={{
                    position: "absolute", top: 10, bottom: 2, width: 10, marginLeft: -5, left: pct(w === "in" ? tin : tout),
                    background: "var(--accent)", borderRadius: 3, cursor: "ew-resize",
                  }} />
                ))}
                <span style={{ position: "absolute", top: 0, bottom: 0, width: 1, left: pct(time), background: "var(--text)", pointerEvents: "none" }} />
              </div>
              <div className="row mono muted" style={{ justifyContent: "space-between", fontSize: 12 }}>
                <span>00:00:00</span>
                <span>{timecode(dur / 2, false)}</span>
                <span>{timecode(dur, false)}</span>
              </div>
            </div>
          </div>

          <div style={{ padding: 20, display: "flex", flexDirection: "column", gap: 24 }}>
            <div className="field">
              <span>Corte no tempo</span>
              <div className="row" style={{ gap: 8 }}>
                <button className="btn btn-sm" onClick={() => setIn(time)}>Marcar entrada <kbd className="mono muted">I</kbd></button>
                <button className="btn btn-sm" onClick={() => setOut(time)}>Saída <kbd className="mono muted">O</kbd></button>
              </div>
            </div>

            <div className="field">
              <span>Recorte do quadro</span>
              <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                <button className="btn btn-sm" onClick={() => setLock((l) => !l)} disabled={!fracRatio}>
                  {lock ? <Lock {...ICON} /> : <LockOpen {...ICON} />}
                  {lock ? "Proporção travada" : "Proporção livre"}
                </button>
                <button className="btn btn-sm" onClick={fitTemplate} disabled={!fracRatio}>Proporção do modelo</button>
              </div>
              <span className="mono muted" style={{ fontSize: 12 }}>
                {Math.round(o.crop_w * dims.w)}×{Math.round(o.crop_h * dims.h)} de {dims.w}×{dims.h}
              </span>
            </div>

            <div className="field">
              <span>Encaixe na área</span>
              <div className="seg">
                {([[null, "Do modelo"], ["cover", "Preencher"], ["contain", "Caber"]] as const).map(([v, l]) => (
                  <button key={l} className={(o.fit_mode ?? null) === v ? "on" : ""} onClick={() => setO((p) => ({ ...p, fit_mode: v }))}>{l}</button>
                ))}
              </div>
            </div>

            <div style={{ marginTop: "auto", display: "flex", flexDirection: "column", gap: 8 }}>
              <button className="btn btn-primary" onClick={() => onSave(o)}>Aplicar</button>
              <button className="btn btn-ghost" onClick={() => setO(FULL)}><RotateCcw {...ICON} />Desfazer ajustes</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export const isAdjusted = (o?: Overrides) =>
  !!o && (o.crop_x !== 0 || o.crop_y !== 0 || o.crop_w !== 1 || o.crop_h !== 1 || o.fit_mode != null || o.trim_start != null || o.trim_end != null);

export { FULL as FULL_FRAME };
