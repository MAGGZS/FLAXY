import { useEffect, useRef, useState } from "react";
import { Check, Cpu, Download, KeyRound, Save } from "lucide-react";
import { api, type AiSettings, type Provider, type WhisperModel } from "../lib/api";
import { useToast } from "../components/Toast";

const ICON = { size: 16, strokeWidth: 1.5 };

const PROVIDERS: { id: Provider; name: string; note: string; key?: keyof AiSettings["keys"] }[] = [
  { id: "local", name: "Whisper local", note: "Roda nesta máquina. Grátis, sem internet depois do download do modelo" },
  { id: "groq", name: "Groq", note: "Whisper large-v3 turbo hospedado. Muito rápido", key: "groq" },
  { id: "openai", name: "OpenAI", note: "whisper-1 com tempo por palavra", key: "openai" },
  { id: "deepgram", name: "Deepgram", note: "Nova-3, bom em português", key: "deepgram" },
  { id: "assemblyai", name: "AssemblyAI", note: "Universal, tempo por palavra", key: "assemblyai" },
];

const LLMS: { id: AiSettings["llm_provider"]; name: string; placeholder: string; key?: keyof AiSettings["keys"] }[] = [
  { id: "anthropic", name: "Anthropic", placeholder: "claude-opus-5-5", key: "anthropic" },
  { id: "openai", name: "OpenAI", placeholder: "nome do modelo", key: "openai" },
  { id: "groq", name: "Groq", placeholder: "llama-3.3-70b-versatile", key: "groq" },
  { id: "ollama", name: "Ollama (local)", placeholder: "qwen2.5:14b" },
];

const LANGS: [string, string][] = [["pt", "Português"], ["en", "Inglês"], ["es", "Espanhol"], ["", "Detectar"]];

export function IA() {
  const toast = useToast();
  const [s, setS] = useState<AiSettings | null>(null);
  const [models, setModels] = useState<WhisperModel[]>([]);
  const [cli, setCli] = useState(true);
  const [dirty, setDirty] = useState(false);
  const poll = useRef<ReturnType<typeof setInterval>>(undefined);

  const loadModels = () =>
    api.get<{ whisper_cli: boolean; models: WhisperModel[] }>("/ai/models").then((r) => {
      setModels(r.models);
      setCli(r.whisper_cli);
      return r.models.some((m) => m.progress !== null);
    });

  useEffect(() => {
    api.get<AiSettings>("/ai/settings").then(setS, (e) => toast("err", e.message));
    loadModels().then((busy) => busy && startPoll());
    return () => clearInterval(poll.current);
  }, []);

  function startPoll() {
    clearInterval(poll.current);
    poll.current = setInterval(() => loadModels().then((busy) => !busy && clearInterval(poll.current)), 1500);
  }

  const set = (p: Partial<AiSettings>) => { setS((c) => c && { ...c, ...p }); setDirty(true); };
  const setKey = (k: keyof AiSettings["keys"], v: string) => s && set({ keys: { ...s.keys, [k]: v } });

  async function save() {
    if (!s) return;
    try {
      setS(await api.put<AiSettings>("/ai/settings", s));
      setDirty(false);
      toast("ok", "Configuração salva");
    } catch (e: any) { toast("err", e.message); }
  }

  async function download(name: string) {
    await api.post(`/ai/models/${name}/download`).catch((e) => toast("err", e.message));
    loadModels();
    startPoll();
  }

  if (!s) return <div className="page"><div className="card empty">Carregando</div></div>;
  const current = PROVIDERS.find((p) => p.id === s.transcriber)!;

  return (
    <div className="page">
      <header className="page-head">
        <div>
          <h1 className="page-title">IA</h1>
          <p className="page-sub">Quem transcreve os vídeos para gerar legenda</p>
        </div>
        <div className="page-actions">
          <button className="btn btn-primary" onClick={save} disabled={!dirty}><Save {...ICON} />Salvar</button>
        </div>
      </header>

      <section style={{ display: "grid", gridTemplateColumns: "minmax(0, 1fr) 380px", gap: 12, alignItems: "start" }}>
        <div className="card">
          <div className="card-head"><span className="card-title">Transcrição</span></div>
          <div className="list">
            {PROVIDERS.map((p) => {
              const on = s.transcriber === p.id;
              const hasKey = p.key ? !!s.keys[p.key] : true;
              return (
                <div key={p.id} className="list-row" style={{ gridTemplateColumns: "24px 160px 1fr 280px", background: on ? "var(--select)" : undefined, cursor: "pointer" }}
                  onClick={() => set({ transcriber: p.id })}>
                  <span style={{ width: 16, height: 16, borderRadius: 8, border: `1px solid ${on ? "var(--accent)" : "var(--line-2)"}`, display: "grid", placeItems: "center" }}>
                    {on && <span style={{ width: 8, height: 8, borderRadius: 4, background: "var(--accent)" }} />}
                  </span>
                  <span style={{ fontWeight: 500 }}>{p.name}</span>
                  <span className="dim" style={{ fontSize: 13 }}>{p.note}</span>
                  {p.key ? (
                    <div className="row" style={{ gap: 8 }} onClick={(e) => e.stopPropagation()}>
                      <KeyRound {...ICON} color={hasKey ? "var(--ok)" : "var(--text-3)"} />
                      <input className="input mono" style={{ flex: 1, height: 32, fontSize: 13 }} type="password" autoComplete="off"
                        placeholder="Chave da API" value={s.keys[p.key]} onChange={(e) => setKey(p.key!, e.target.value)} />
                    </div>
                  ) : <span className="muted" style={{ fontSize: 13 }}>{cli ? "whisper-cli pronto" : "whisper-cli ausente"}</span>}
                </div>
              );
            })}
          </div>
          <div className="card-pad row" style={{ borderTop: "1px solid var(--line)", gap: 16 }}>
            <label className="field" style={{ width: 200 }}>
              <span>Idioma dos vídeos</span>
              <select className="select" value={s.language} onChange={(e) => set({ language: e.target.value })}>
                {LANGS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
              </select>
            </label>
            <span className="muted" style={{ fontSize: 13, alignSelf: "flex-end", paddingBottom: 8 }}>
              Em uso: {current.name}. As chaves ficam salvas só nesta máquina
            </span>
          </div>
        </div>

        <div className="card" style={{ gridColumn: 1 }}>
          <div className="card-head">
            <span className="card-title">Análise de retenção</span>
            <span className="meta">LLM que escolhe os melhores trechos na página Clipes</span>
          </div>
          <div className="card-pad" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <div className="field">
              <span>Provedor</span>
              <div className="seg">
                {LLMS.map((l) => <button key={l.id} className={s.llm_provider === l.id ? "on" : ""} onClick={() => set({ llm_provider: l.id })}>{l.name}</button>)}
              </div>
            </div>
            {(() => {
              const cur = LLMS.find((l) => l.id === s.llm_provider);
              if (!cur) return <span className="muted" style={{ fontSize: 13 }}>Escolha um provedor para usar a página Clipes</span>;
              return (
                <div className="grid-2">
                  <label className="field">
                    <span>Modelo{cur.id === "anthropic" ? " (vazio usa o padrão)" : ""}</span>
                    <input className="input mono" value={s.llm_model} placeholder={cur.placeholder} onChange={(e) => set({ llm_model: e.target.value })} />
                  </label>
                  {cur.key ? (
                    <label className="field">
                      <span>Chave da {cur.name}{cur.key !== "anthropic" ? " (a mesma da transcrição)" : ""}</span>
                      <input className="input mono" type="password" autoComplete="off" value={s.keys[cur.key]} placeholder="Chave da API" onChange={(e) => setKey(cur.key!, e.target.value)} />
                    </label>
                  ) : (
                    <label className="field">
                      <span>Endereço do Ollama</span>
                      <input className="input mono" value={s.ollama_url} placeholder="http://127.0.0.1:11434" onChange={(e) => set({ ollama_url: e.target.value })} />
                    </label>
                  )}
                </div>
              );
            })()}
          </div>
        </div>

        <aside className="card" style={{ gridColumn: 2, gridRow: "1 / span 2" }}>
          <div className="card-head">
            <Cpu {...ICON} color="var(--text-3)" />
            <span className="card-title">Modelos do Whisper local</span>
          </div>
          <div className="list">
            {models.map((m) => {
              const on = s.local_model === m.name;
              return (
                <div key={m.name} className="list-row" style={{ gridTemplateColumns: "1fr auto", minHeight: 56 }}>
                  <div style={{ display: "flex", flexDirection: "column", gap: 4, minWidth: 0 }}>
                    <span className="mono" style={{ fontSize: 13 }}>{m.name}{m.name === "large-v3-turbo" && <span className="muted"> · recomendado</span>}</span>
                    {m.progress !== null ? (
                      <div className="progress"><i style={{ width: `${m.progress}%` }} /></div>
                    ) : (
                      <span className="mono muted" style={{ fontSize: 12 }}>
                        {m.size_mb.toLocaleString("pt-BR")} MB{m.error ? <span style={{ color: "var(--err)" }}> · {m.error}</span> : ""}
                      </span>
                    )}
                  </div>
                  {m.installed ? (
                    <button className={`btn btn-sm${on ? " btn-primary" : ""}`} onClick={() => set({ local_model: m.name })}>
                      {on ? <><Check {...ICON} />Em uso</> : "Usar"}
                    </button>
                  ) : m.progress !== null ? (
                    <span className="mono muted" style={{ fontSize: 12 }}>{m.progress}%</span>
                  ) : (
                    <button className="btn btn-sm" onClick={() => download(m.name)}><Download {...ICON} />Baixar</button>
                  )}
                </div>
              );
            })}
          </div>
        </aside>
      </section>
    </div>
  );
}
