# agente.md — Requisitos e regras do FLAXY v2

Este arquivo é a fonte de verdade para qualquer agente (humano ou IA) que trabalhe
no FLAXY. Leia antes de qualquer mudança. Se algo aqui conflitar com o código, o
arquivo vence até que o dono do projeto decida o contrário.

## Regras de trabalho (obrigatórias)

1. **Não tomar decisões importantes sozinho.** Stack, provedores de IA, modelo de
   cobrança, banco de dados, hospedagem, layout de páginas e qualquer coisa difícil
   de reverter exigem confirmação explícita do dono do projeto.
2. **Perguntar quando houver dúvida.** Melhor uma pergunta a mais do que uma
   implementação errada.
3. **Registrar decisões.** Toda decisão confirmada entra na seção "Decisões
   tomadas" abaixo, com data. Pendências ficam em "Decisões pendentes".
4. **Comunicação em português.** Código, nomes de API e termos técnicos podem
   ficar em inglês.

## Contexto

FLAXY é um editor de vídeos em massa. Hoje: FastAPI + FFmpeg (backend, Render) e
React + Vite (frontend, Vercel). Fluxo atual: template (fundo + área do vídeo) →
lote de até 100 vídeos → recorte individual opcional → resultados/ZIP.

Referências de produto analisadas (páginas públicas, novembro de 2026):
- **automadark.ai** — editor em lote com fundo 1080×1920, logo/marca-d'água,
  cortes com IA a partir de link, textos por IA, agendamento, métricas. Planos
  por saldo de vídeos/ciclo (Start R$ 97 / 300, Pro R$ 197 / 1.500,
  Scale R$ 297 / 5.000) e créditos de corte separados (1 crédito ≈ 1 min de vídeo
  original). Saldo reservado no envio do lote; falhas devolvem saldo.
- **opus.pro** — link do YouTube → IA escolhe melhores momentos → reframe com
  rastreamento (ReframeAnything) → legendas automáticas editáveis → brand kit
  (fonte, cor, logo, intro/outro) → exportar/publicar.

## Requisitos do v2

### R1. Reescrita em Rust
- Todo o backend reescrito em Rust, o mais otimizado possível.
- Escopo exato (só backend ou também frontend em Rust/WASM, web ou desktop):
  ver "Decisões pendentes".

### R2. Frontend novo do zero
- Visual totalmente novo, distinto do atual. Nada de reaproveitar o layout atual.
- Paleta: **cinza, azul escuro e cinza escuro**. O roxo `#7c5cfc` atual sai.
- A skill `/unslop` guia todas as decisões de front-end (direção, espaçamento,
  cor, ícones, tipografia, layout, movimento).
- Páginas modeladas e reconstruídas do zero.

### R3. Aba de IA para legendas automáticas
- Nova aba para o usuário cadastrar chaves de API de IA.
- Legendagem automática (transcrição com timestamps por palavra).
- **Modelos de legenda** (estilos): fonte, cor, destaque de palavra, posição,
  animação. Aplicáveis em lote.
- Legendas editáveis antes do render.

### R4. Áudio
- Adicionar faixa de áudio aos vídeos (música/narração), com controle de volume
  e mixagem com o áudio original.

### R5. Corte individual de vídeo
- Cortar (trim: início/fim) cada vídeo individualmente, além do recorte de
  enquadramento que já existe.

### R6. Clipes de retenção (estilo Opus)
1. Usuário cola link do YouTube.
2. Sistema baixa o vídeo pela ferramenta de linha de comando (yt-dlp).
3. Transcreve o vídeo.
4. IA analisa a transcrição e identifica os trechos de melhor retenção.
5. Usuário revisa e **aprova** os clipes sugeridos.
6. Só após a aprovação o sistema corta e renderiza.
7. Modelos de clipe: **vertical (9:16) ou horizontal (16:9)**.
8. Detecção de **câmera (rosto/pessoa)** e de **tela** (gravação de tela) para
   montar o layout (ex.: câmera em cima, tela embaixo no vertical).

### R7. Planos
- Reavaliar a estrutura de planos tomando automadark.ai como base (saldo de
  vídeos por ciclo + créditos de corte por minuto). Valores e escopo: pendente.

## Decisões tomadas

- 2026-11-13 — **Plataforma: web e desktop.** Núcleo Rust compartilhado, servido
  por um servidor web e por um app desktop (Tauri).
- 2026-11-13 — **Frontend: React + TypeScript, recriado do zero.** Rust fica no
  backend/núcleo. O mesmo frontend serve web e Tauri.
- 2026-11-13 — **IA: local + API.** Whisper local (whisper.cpp) como padrão para
  transcrição; provedores por API opcionais, cadastrados na aba de IA.
- 2026-11-13 — **Sem planos.** Projeto pessoal: sem contas pagas, saldo ou
  cobrança. R7 fica fora do escopo.

- 2026-11-13 — **Detecção de câmera/tela: manual + sugestão.** O usuário marca as
  áreas de câmera e de tela; a IA apenas sugere. Nada de detecção automática
  obrigatória.
- 2026-11-13 — **LLM de retenção: plugável, sem padrão fixo.** O usuário escolhe o
  provedor na aba de IA.
- 2026-11-13 — **Banco: SQLite em ambos** (desktop e servidor), via `sqlx`.
  Arquivos de mídia em disco.
- 2026-11-13 — **Ordem de entrega:**
  1. Núcleo Rust + novo frontend com paridade do editor em massa atual.
  2. Corte individual (trim) + áudio.
  3. Legendas por IA + modelos de legenda.
  4. Clipes de retenção (YouTube → transcrição → aprovação → render).

- 2026-11-13 — **Prioridade: app desktop `.exe`** (Tauri v2). O dono quer "um
  aplicativo que abre como aplicativo". A nuvem continua possível no futuro com
  o mesmo núcleo (`flaxy-server`), mas não é foco agora. Render/Vercel saem do
  plano imediato.
- 2026-11-13 — **Janela: Tauri** (Rust nativo + WebView2), reaproveitando o
  frontend React. UI 100% Rust (Slint/egui) foi descartada.
- 2026-11-13 — **Arquitetura confirmada** (abaixo).
- 2026-11-13 — **Páginas:** Início, Lotes, Biblioteca, Modelos (abas: fundos,
  estilos de legenda, layouts de clipe), Clipes, IA, Configurações.
- 2026-11-13 — **Hospedagem web: Render** (Docker + disco persistente); o Axum
  serve o frontend.

- 2026-11-13 — **Direção visual aprovada** (via `/unslop`):
  - Produto: estação de produção que transforma lotes de vídeos brutos e vídeos
    longos em clipes prontos, para quem opera páginas de conteúdo sozinho.
  - Caráter: industrial, exato, noturno.
  - Mundo: ilha de edição à noite (monitor de referência, timecode, régua de
    timeline, mesa de corte).
  - Momentos: (1) timeline com frames, waveform do áudio e curva de retenção em
    azul; (2) prévia 9:16 do resultado com a legenda.
  - Assinatura: timecode em mono (`00:01:24:12`) e régua com ticks em toda
    duração, posição e progresso.
  - Paleta: `#0e0f11` lateral, `#131416` fundo, `#191a1d` cartão, `#222429`
    ativo/inputs, `#16213d` seleção, `#3b63d6` acento; texto `#eceef1`,
    `#a2a6ae`, `#7d828c`.
  - Tipo: Geist + Geist Mono. Raios 8/12/16. Ícones lucide 16px, traço 1.5.
  - Proibido: brilho, gradiente de cor, emoji, rótulos em caixa alta espaçada.

- 2026-11-13 — **Só navegador. App desktop removido.** Substitui as decisões
  "Prioridade: app desktop `.exe`" e "Janela: Tauri". O dono pediu para tirar
  tudo que exige instalar algo no PC além das dependências necessárias. Saíram
  `crates/flaxy-desktop` (Tauri), `build-desktop.bat`, `start-desktop.bat`,
  `tools/fetch-*.ps1` e `@tauri-apps/cli`. O FLAXY roda como `flaxy-server`,
  que serve a API e o `web/dist` em http://127.0.0.1:8000. FFmpeg, yt-dlp e
  whisper-cli ficam em `bin/` (fora do git) ou no PATH.

## Arquitetura

```
FLAXY/
├── crates/
│   ├── flaxy-core/      # domínio, FFmpeg, fila, SQLite, IA, yt-dlp
│   └── flaxy-server/    # Axum: HTTP + WebSocket, serve o frontend
├── web/                 # React + TS novo, aberto no navegador
├── bin/                 # ffmpeg, yt-dlp, whisper-cli locais (opcional, fora do git)
└── agente.md
```

- O frontend fala com uma única API HTTP, servida na mesma origem.
- FFmpeg chamado via `tokio::process`. Progresso por `-progress pipe:1`, não por
  regex no stderr.
- Encoder de GPU (NVENC/QSV/AMF) detectado uma vez na inicialização, não a cada
  vídeo como hoje.
- Um único filtergraph por vídeo: enquadramento + legenda (`ass`) + mixagem de
  áudio + trim. Uma passada de encode.
- Concorrência configurável (sessões de GPU), fila persistida no SQLite.
- Traits `Transcriber` (whisper.cpp local, APIs) e `LlmProvider` (Anthropic,
  OpenAI, Ollama, compatíveis com OpenAI).
- Download do YouTube por `yt-dlp` em linha de comando.

## Andamento

### Fase 1: núcleo Rust + frontend novo (em andamento)
- [x] `crates/flaxy-core`: config, SQLite (WAL, migrações embutidas), templates,
      composição FFmpeg (recorte, trim, mixagem, encoder de GPU testado na partida,
      progresso por `-progress`), fila persistida com concorrência global.
- [x] `crates/flaxy-server`: API Axum em `/api` (paridade com o FastAPI + `/stats`),
      WebSocket de progresso, Range em vídeos, ZIP sem compressão, serve o `web/dist`.
- [x] `web/`: React + TS do zero. Páginas Início, Lotes (com editor de recorte e
      trim), Biblioteca, Modelos (fundos), Configurações.
- [x] Teste ponta a ponta: modelo + upload + lote com recorte e trim renderizado (AMF).
- [-] App desktop Tauri (`FLAXY.exe`): removido em 2026-11-13 (só navegador).
- [ ] Nuvem (futuro): Dockerfile Rust usando `flaxy-server`.
- [ ] Remover/arquivar `backend/` e `frontend/` antigos (decisão do dono).

Rodar: `start-v2.bat` (abre http://127.0.0.1:8000 no navegador).
Dev do front: `cd web && npm run dev`.

### Fase 2: trim + áudio
- [x] Trim por vídeo (já entregue na fase 1).
- [x] Faixa de áudio extra (música/narração) por lote: volume da faixa, volume
      do original (até mudo), início da faixa, fade de 2s no fim, loop se curta.
      Gravada nos overrides de cada item (dá para ter faixa por vídeo depois).
      Rota `POST /api/upload-audio`. Testado ponta a ponta.

Commits: só quando o dono mandar.

### Fase 3: legendas por IA (aba IA, Whisper local + APIs, modelos de legenda)
Decisões (2026-11-13):
- Whisper local: executável `whisper-cli` (whisper.cpp) em `bin/` ou no PATH. Modelo baixado pelo app na primeira vez.
- Modelo padrão: `large-v3-turbo`.
- APIs suportadas: Groq, OpenAI, Deepgram, AssemblyAI (chave na aba IA).
- Estilos prontos: palavra destacada, palavra por palavra, clássica, caixa colorida.
  O usuário pode criar outros.
- Legenda queimada no vídeo via arquivo ASS (libass), gerado na hora do render
  com os tempos ajustados ao trim.

Entregue:
- [x] `flaxy-core`: `ai.rs` (configuração, Whisper local, Groq, OpenAI, Deepgram,
      AssemblyAI), `transcripts.rs` (fila, uma por vez), `captions.rs` (estilos +
      geração ASS dos 4 tipos). Migração `0002_captions.sql`.
- [x] API: `/api/ai/settings`, `/api/ai/models` (+download com progresso),
      `/api/transcripts`, `/api/caption-styles`.
- [x] Front: página IA, aba Modelos > Legendas (editor + prévia), painel de
      legenda no Lote, editor de texto da transcrição.
- [x] Render dos 4 estilos testado com palavras digitadas (trim conferido).
- [ ] Testar transcrição real (falta whisper-cli + modelo, ou uma chave de API).
### Fase 4: clipes de retenção (yt-dlp, transcrição, aprovação, layouts)
Decisões (2026-11-13):
- Duração: o usuário escolhe a faixa (15–30s, 30–60s, 60–90s, automático).
- Até 10 sugestões por vídeo, com nota 0–100 de retenção e justificativa.
- LLM plugável: Anthropic (padrão `claude-opus-5-5`, saída JSON estruturada,
  fallback server-side ligado), OpenAI, Groq, Ollama. Escolha na aba IA.
- Layouts prontos: vertical só câmera, vertical tela em cima + câmera embaixo
  (60/40), horizontal original.
- Câmera/tela: a IA sugere as áreas, o usuário corrige (ver decisão anterior).

Entregue:
- [x] `clips.rs`: yt-dlp (link -> mp4 até 1080p), transcrição, análise por LLM,
      ajuste das sugestões às palavras, render por layout com legenda opcional.
      `llm.rs`: os 4 provedores. Migração `0003_clips.sql`.
- [x] API `/api/clips` (+ `/render`, `/source`, `/tools`).
- [x] Página Clipes: link, faixa, layout, mapa de retenção no tempo, aprovação,
      prévia do trecho, editor de áreas de câmera/tela, legenda, download.
- [x] Testado: download real pelo yt-dlp; render dos 3 layouts com legenda.
- [ ] Testar análise real por LLM (falta chave/modelo configurado).

## Decisões pendentes

- Direção visual do novo frontend (definida com `/unslop`, aprovada pelo dono).
- Destino do código Python/React atual após paridade (apagar ou arquivar).
