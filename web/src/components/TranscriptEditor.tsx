import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { api, type Transcript, type Word } from "../lib/api";
import { timecode } from "../lib/format";
import { useToast } from "./Toast";

/** Edita o texto de cada palavra sem mexer nos tempos. Frases separadas por pausa. */
export function TranscriptEditor({ id, name, onClose }: { id: number; name: string; onClose: () => void }) {
  const toast = useToast();
  const [words, setWords] = useState<Word[] | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    api.get<Transcript>(`/transcripts/${id}`).then((t) => setWords(t.words), (e) => toast("err", e.message));
  }, [id]);

  async function save() {
    if (!words) return;
    try {
      await api.put(`/transcripts/${id}`, { words: words.filter((w) => w.text.trim()) });
      toast("ok", "Legenda salva");
      onClose();
    } catch (e: any) { toast("err", e.message); }
  }

  // Agrupa em linhas por pausa > 0,6 s ou fim de frase.
  const lines: number[][] = [];
  (words ?? []).forEach((w, i) => {
    const prev = words![i - 1];
    if (!prev || w.start - prev.end > 0.6 || /[.!?]$/.test(prev.text)) lines.push([]);
    lines[lines.length - 1].push(i);
  });

  return (
    <div className="overlay" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" style={{ width: "min(860px, 100%)" }} role="dialog" aria-label={`Legenda de ${name}`}>
        <div className="card-head">
          <span className="card-title">Legenda · {name}</span>
          <span className="meta mono">{words?.length ?? 0} palavras</span>
          <button className="btn btn-ghost btn-icon btn-sm" onClick={onClose} aria-label="Fechar"><X size={16} strokeWidth={1.5} /></button>
        </div>
        <div className="list" style={{ maxHeight: "60vh", overflowY: "auto" }}>
          {words === null && <div className="empty">Carregando</div>}
          {words?.length === 0 && <div className="empty">Nenhuma fala encontrada</div>}
          {lines.map((idxs) => (
            <div key={idxs[0]} className="list-row" style={{ gridTemplateColumns: "96px 1fr", alignItems: "start", padding: "10px 20px" }}>
              <span className="mono muted" style={{ fontSize: 12, paddingTop: 6 }}>{timecode(words![idxs[0]].start)}</span>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {idxs.map((i) => (
                  <input key={i} className="input" value={words![i].text}
                    style={{ height: 28, padding: "0 6px", width: `${Math.max(3, words![i].text.length + 1)}ch`, fontSize: 13 }}
                    onChange={(e) => { const v = e.target.value; setWords((ws) => ws!.map((w, j) => (j === i ? { ...w, text: v } : w))); setDirty(true); }} />
                ))}
              </div>
            </div>
          ))}
        </div>
        <div className="card-pad row" style={{ borderTop: "1px solid var(--line)", justifyContent: "space-between" }}>
          <span className="muted" style={{ fontSize: 13 }}>Apague o texto de uma palavra para tirá-la da legenda</span>
          <div className="row" style={{ gap: 8 }}>
            <button className="btn btn-ghost" onClick={onClose}>Cancelar</button>
            <button className="btn btn-primary" onClick={save} disabled={!dirty}>Salvar legenda</button>
          </div>
        </div>
      </div>
    </div>
  );
}
