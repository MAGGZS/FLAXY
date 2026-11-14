import { useEffect, useState } from "react";
import { Copy, Plus, Save, Trash2, Type } from "lucide-react";
import { api, type CaptionStyle, type StyleRow } from "../lib/api";
import { useToast } from "./Toast";

const ICON = { size: 16, strokeWidth: 1.5 };

const SAMPLE = ["Isso", "aqui", "muda", "tudo", "na", "sua", "página"];

const KINDS: [CaptionStyle["kind"], string][] = [
  ["highlight", "Palavra destacada"],
  ["word", "Palavra por palavra"],
  ["classic", "Clássica"],
  ["box", "Caixa colorida"],
];

const FONTS = ["Arial", "Arial Black", "Impact", "Verdana", "Segoe UI", "Trebuchet MS", "Georgia", "Montserrat", "Poppins"];

/** Pré-visualização em CSS de como o ASS vai ficar (aproximação fiel o bastante para escolher). */
export function CaptionPreview({ st, height = 420 }: { st: CaptionStyle; height?: number }) {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => setTick((v) => v + 1), 420);
    return () => clearInterval(t);
  }, []);

  const scale = height / 1920;
  const per = st.kind === "word" ? 1 : st.kind === "classic" ? Math.min(st.max_words, 7) : Math.max(1, st.max_words);
  const idx = tick % SAMPLE.length;
  const start = Math.floor(idx / per) * per;
  const chunk = SAMPLE.slice(start, start + per);
  const active = idx - start;
  const t = (w: string) => (st.uppercase ? w.toUpperCase() : w);
  const stroke = st.outline * scale;
  const pos: React.CSSProperties =
    st.position === "top" ? { top: `${st.margin * 100}%` } :
    st.position === "center" ? { top: "50%", transform: "translateY(-50%)" } :
    { bottom: `${st.margin * 100}%` };

  const base: React.CSSProperties = {
    fontFamily: `"${st.font}", Arial, sans-serif`,
    fontSize: st.size * scale,
    fontWeight: st.bold ? 800 : 400,
    color: st.color,
    lineHeight: 1.15,
    WebkitTextStroke: stroke > 0 ? `${stroke}px ${st.outline_color}` : undefined,
    paintOrder: "stroke fill",
  };
  const back = `${st.back_color}${Math.round(st.back_opacity * 255).toString(16).padStart(2, "0")}`;

  return (
    <div style={{ position: "relative", height, aspectRatio: "9 / 16", borderRadius: "var(--r)", overflow: "hidden", margin: "0 auto",
      background: "linear-gradient(160deg, #2a2d33 0%, #16181c 60%, #0e0f11 100%)" }}>
      <div style={{ position: "absolute", left: "6%", right: "6%", textAlign: "center", ...pos }}>
        {st.kind === "classic" ? (
          <span style={{ ...base, background: back, padding: `${2 * scale * 10}px ${4 * scale * 10}px`, boxDecorationBreak: "clone", WebkitBoxDecorationBreak: "clone" }}>
            {chunk.map(t).join(" ")}
          </span>
        ) : (
          <span style={base}>
            {chunk.map((w, i) => {
              const on = i === active;
              const style: React.CSSProperties =
                st.kind === "highlight" && on ? { color: st.highlight } :
                st.kind === "box" && on ? { background: st.highlight, WebkitTextStroke: undefined, padding: `0 ${st.size * scale * 0.18}px`, borderRadius: 2 } :
                st.kind === "word" ? { display: "inline-block", animation: "pop 120ms var(--ease)" } : {};
              return <span key={`${start}-${i}-${tick}`} style={style}>{t(w)}{i < chunk.length - 1 ? " " : ""}</span>;
            })}
          </span>
        )}
      </div>
    </div>
  );
}

export function CaptionStyles() {
  const toast = useToast();
  const [list, setList] = useState<StyleRow[]>([]);
  const [sel, setSel] = useState<StyleRow | null>(null);
  const [dirty, setDirty] = useState(false);

  const load = (pick?: number) =>
    api.get<StyleRow[]>("/caption-styles").then((r) => {
      setList(r);
      setSel((cur) => r.find((x) => x.id === (pick ?? cur?.id)) ?? r[0] ?? null);
      setDirty(false);
    }, (e) => toast("err", e.message));
  useEffect(() => { load(); }, []);

  const set = (p: Partial<CaptionStyle>) => { if (sel) { setSel({ ...sel, style: { ...sel.style, ...p } }); setDirty(true); } };

  async function save() {
    if (!sel) return;
    try {
      // Estilo pronto não é sobrescrito: salvar cria uma cópia editável.
      if (sel.builtin) {
        const r = await api.post<StyleRow>("/caption-styles", { name: `${sel.name} (meu)`, style: sel.style });
        load(r.id);
      } else {
        await api.put(`/caption-styles/${sel.id}`, { name: sel.name, style: sel.style });
        load(sel.id);
      }
      toast("ok", "Estilo salvo");
    } catch (e: any) { toast("err", e.message); }
  }

  async function duplicate() {
    if (!sel) return;
    const r = await api.post<StyleRow>("/caption-styles", { name: `${sel.name} (cópia)`, style: sel.style }).catch((e) => { toast("err", e.message); return null; });
    if (r) load(r.id);
  }

  async function remove() {
    if (!sel || sel.builtin || !confirm(`Apagar o estilo "${sel.name}"?`)) return;
    await api.del(`/caption-styles/${sel.id}`).catch((e) => toast("err", e.message));
    load(-1);
  }

  if (!sel) return <div className="card empty"><Type size={20} strokeWidth={1.5} />Nenhum estilo</div>;
  const s = sel.style;

  return (
    <section style={{ display: "grid", gridTemplateColumns: "240px 1fr 320px", gap: 12, alignItems: "start" }}>
      <div className="card">
        <div className="card-head">
          <span className="card-title">Estilos</span>
          <button className="btn btn-ghost btn-icon btn-sm" style={{ marginLeft: "auto" }} onClick={duplicate} aria-label="Novo a partir deste"><Plus {...ICON} /></button>
        </div>
        <div className="list">
          {list.map((r) => (
            <button key={r.id} className="list-row" onClick={() => { setSel(r); setDirty(false); }}
              style={{ gridTemplateColumns: "1fr auto", minHeight: 40, textAlign: "left", background: r.id === sel.id ? "var(--select)" : undefined }}>
              <span style={{ fontSize: 13 }}>{r.name}</span>
              {r.builtin && <span className="mono muted" style={{ fontSize: 11 }}>pronto</span>}
            </button>
          ))}
        </div>
      </div>

      <div className="card">
        <div className="card-head">
          <input className="input" style={{ height: 32, flex: 1, maxWidth: 320 }} value={sel.name} disabled={sel.builtin}
            onChange={(e) => { setSel({ ...sel, name: e.target.value }); setDirty(true); }} aria-label="Nome do estilo" />
          <div className="row" style={{ marginLeft: "auto", gap: 4 }}>
            <button className="btn btn-ghost btn-icon btn-sm" onClick={duplicate} aria-label="Duplicar"><Copy {...ICON} /></button>
            {!sel.builtin && <button className="btn btn-ghost btn-icon btn-sm btn-danger" onClick={remove} aria-label="Apagar"><Trash2 {...ICON} /></button>}
            <button className="btn btn-primary btn-sm" onClick={save} disabled={!dirty}><Save {...ICON} />{sel.builtin ? "Salvar como meu" : "Salvar"}</button>
          </div>
        </div>
        <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
          <div className="field">
            <span>Tipo</span>
            <div className="seg">{KINDS.map(([k, l]) => <button key={k} className={s.kind === k ? "on" : ""} onClick={() => set({ kind: k })}>{l}</button>)}</div>
          </div>
          <div className="grid-2">
            <label className="field"><span>Fonte</span>
              <select className="select" value={s.font} onChange={(e) => set({ font: e.target.value })}>
                {[...new Set([s.font, ...FONTS])].map((f) => <option key={f}>{f}</option>)}
              </select>
            </label>
            <label className="field"><span>Tamanho <span className="mono">{s.size}px</span></span>
              <input type="range" min={32} max={180} step={2} value={s.size} onChange={(e) => set({ size: Number(e.target.value) })} />
            </label>
          </div>
          <div className="row" style={{ gap: 8 }}>
            <button className={`btn btn-sm${s.bold ? " btn-primary" : ""}`} onClick={() => set({ bold: !s.bold })}>Negrito</button>
            <button className={`btn btn-sm${s.uppercase ? " btn-primary" : ""}`} onClick={() => set({ uppercase: !s.uppercase })}>Caixa alta</button>
          </div>
          <div className="grid-2" style={{ gridTemplateColumns: "repeat(3, 1fr)" }}>
            {([["color", "Texto"], ["highlight", s.kind === "box" ? "Caixa" : "Destaque"], ["outline_color", "Contorno"]] as const).map(([k, l]) => (
              <label key={k} className="field"><span>{l}</span>
                <div className="row" style={{ gap: 8 }}>
                  <input type="color" value={s[k]} onChange={(e) => set({ [k]: e.target.value })} style={{ width: 36, height: 32, border: "1px solid var(--line)", borderRadius: 6, background: "none", padding: 2 }} />
                  <span className="mono muted" style={{ fontSize: 12 }}>{s[k]}</span>
                </div>
              </label>
            ))}
          </div>
          <label className="field"><span>Contorno <span className="mono">{s.outline}px</span></span>
            <input type="range" min={0} max={16} step={1} value={s.outline} onChange={(e) => set({ outline: Number(e.target.value) })} />
          </label>
          {s.kind === "classic" && (
            <div className="grid-2">
              <label className="field"><span>Cor do fundo</span>
                <input type="color" value={s.back_color} onChange={(e) => set({ back_color: e.target.value })} style={{ width: 36, height: 32, border: "1px solid var(--line)", borderRadius: 6, background: "none", padding: 2 }} />
              </label>
              <label className="field"><span>Opacidade <span className="mono">{Math.round(s.back_opacity * 100)}%</span></span>
                <input type="range" min={0} max={1} step={0.05} value={s.back_opacity} onChange={(e) => set({ back_opacity: Number(e.target.value) })} />
              </label>
            </div>
          )}
          <div className="grid-2">
            <div className="field"><span>Posição</span>
              <div className="seg">{([["top", "Topo"], ["center", "Centro"], ["bottom", "Base"]] as const).map(([k, l]) => <button key={k} className={s.position === k ? "on" : ""} onClick={() => set({ position: k })}>{l}</button>)}</div>
            </div>
            <label className="field"><span>Distância da borda <span className="mono">{Math.round(s.margin * 100)}%</span></span>
              <input type="range" min={0} max={0.45} step={0.01} value={s.margin} disabled={s.position === "center"} onChange={(e) => set({ margin: Number(e.target.value) })} />
            </label>
          </div>
          {s.kind !== "word" && (
            <div className="grid-2">
              <label className="field"><span>Palavras por vez <span className="mono">{s.max_words}</span></span>
                <input type="range" min={1} max={10} step={1} value={s.max_words} onChange={(e) => set({ max_words: Number(e.target.value) })} />
              </label>
              <label className="field"><span>Caracteres por linha <span className="mono">{s.max_chars}</span></span>
                <input type="range" min={8} max={60} step={1} value={s.max_chars} onChange={(e) => set({ max_chars: Number(e.target.value) })} />
              </label>
            </div>
          )}
        </div>
      </div>

      <aside className="card" style={{ position: "sticky", top: 24 }}>
        <div className="card-head"><span className="card-title">Prévia</span><span className="meta mono">9:16</span></div>
        <div className="card-pad"><CaptionPreview st={s} /></div>
      </aside>
    </section>
  );
}
