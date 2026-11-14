import { useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { Lock, LockOpen, X } from "lucide-react";
import type { ClipLayout, Rect } from "../lib/api";

const ICON = { size: 16, strokeWidth: 1.5 };
const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

type Box = { key: "camera" | "screen"; label: string; color: string; ratio: number | null };

/** Proporção (largura/altura em px de saída) de cada área por layout. */
function boxes(layout: ClipLayout): Box[] {
  if (layout === "vertical_split")
    return [
      { key: "screen", label: "Tela · em cima", color: "var(--text)", ratio: 1080 / 1152 },
      { key: "camera", label: "Câmera · embaixo", color: "var(--accent)", ratio: 1080 / 768 },
    ];
  if (layout === "vertical_camera") return [{ key: "camera", label: "Câmera", color: "var(--accent)", ratio: 9 / 16 }];
  return [];
}

/**
 * Marca câmera e tela sobre um quadro do vídeo. A sugestão vem do servidor;
 * o usuário só corrige. Proporção travada evita que o recorte distorça.
 */
export function RectEditor({
  src, at, srcW, srcH, layout, camera, screen, onSave, onClose,
}: {
  src: string;
  at: number;
  srcW: number;
  srcH: number;
  layout: ClipLayout;
  camera: Rect;
  screen: Rect;
  onSave: (camera: Rect, screen: Rect) => void;
  onClose: () => void;
}) {
  const [rects, setRects] = useState({ camera, screen });
  const [lock, setLock] = useState(true);
  const stage = useRef<HTMLDivElement>(null);
  const srcAr = srcW > 0 && srcH > 0 ? srcW / srcH : 16 / 9;
  const list = boxes(layout);

  function drag(b: Box, mode: "move" | "nw" | "se", e: RPointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    const r0 = rects[b.key];
    const rect = stage.current!.getBoundingClientRect();
    const sx = e.clientX, sy = e.clientY;
    // Proporção em fração da fonte: (px w / px h) / (fonte w / fonte h).
    const fr = b.ratio ? b.ratio / srcAr : null;
    const mv = (ev: PointerEvent) => {
      const dx = (ev.clientX - sx) / rect.width, dy = (ev.clientY - sy) / rect.height;
      let { x, y, w, h } = r0;
      if (mode === "move") {
        x = clamp(x + dx, 0, 1 - w);
        y = clamp(y + dy, 0, 1 - h);
      } else {
        if (mode === "se") { w = clamp(w + dx, 0.05, 1 - x); h = clamp(h + dy, 0.05, 1 - y); }
        else { const nx = clamp(x + dx, 0, x + w - 0.05), ny = clamp(y + dy, 0, y + h - 0.05); w += x - nx; h += y - ny; x = nx; y = ny; }
        if (lock && fr) {
          h = w / fr;
          if (y + h > 1) { h = 1 - y; w = h * fr; }
          if (x + w > 1) { w = 1 - x; h = w / fr; }
        }
      }
      setRects((cur) => ({ ...cur, [b.key]: { x, y, w, h } }));
    };
    const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  }

  return (
    <div className="overlay" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-label="Marcar câmera e tela">
        <div className="card-head">
          <span className="card-title">Marcar câmera e tela</span>
          <span className="meta">A IA sugeriu as áreas. Arraste para corrigir</span>
          <button className="btn btn-ghost btn-icon btn-sm" onClick={onClose} aria-label="Fechar"><X {...ICON} /></button>
        </div>
        <div style={{ padding: 20, display: "flex", flexDirection: "column", gap: 16 }}>
          <div style={{ background: "#000", borderRadius: "var(--r)", display: "grid", placeItems: "center", height: "min(60vh, 600px)" }}>
            <div ref={stage} style={{ position: "relative", aspectRatio: `${srcAr}`, height: "100%", maxWidth: "100%" }}>
              <video src={`${src}#t=${at}`} muted preload="auto" style={{ width: "100%", height: "100%", objectFit: "contain" }} />
              {list.map((b) => {
                const r = rects[b.key];
                return (
                  <div key={b.key} onPointerDown={(e) => drag(b, "move", e)} style={{
                    position: "absolute", left: `${r.x * 100}%`, top: `${r.y * 100}%`, width: `${r.w * 100}%`, height: `${r.h * 100}%`,
                    outline: `2px solid ${b.color}`, background: "rgba(5,6,8,0.15)", cursor: "move",
                  }}>
                    <span className="mono" style={{ position: "absolute", left: 4, top: 4, fontSize: 12, padding: "2px 6px", background: "rgba(5,6,8,0.8)", borderRadius: 4 }}>{b.label}</span>
                    {(["nw", "se"] as const).map((m) => (
                      <span key={m} onPointerDown={(e) => drag(b, m, e)} style={{
                        position: "absolute", width: 12, height: 12, background: b.color, borderRadius: 2, cursor: `${m}-resize`,
                        [m === "nw" ? "left" : "right"]: -6, [m === "nw" ? "top" : "bottom"]: -6,
                      }} />
                    ))}
                  </div>
                );
              })}
            </div>
          </div>
          <div className="row" style={{ justifyContent: "space-between" }}>
            <button className="btn btn-sm" onClick={() => setLock((l) => !l)}>
              {lock ? <Lock {...ICON} /> : <LockOpen {...ICON} />}{lock ? "Proporção travada" : "Proporção livre"}
            </button>
            <div className="row" style={{ gap: 8 }}>
              <button className="btn btn-ghost" onClick={onClose}>Cancelar</button>
              <button className="btn btn-primary" onClick={() => onSave(rects.camera, rects.screen)}>Aplicar</button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
