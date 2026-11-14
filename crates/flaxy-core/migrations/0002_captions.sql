-- Configurações chave/valor (provedores de IA, chaves, modelo local).
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Estilos de legenda. `data` é o JSON de CaptionStyle.
CREATE TABLE IF NOT EXISTS caption_styles (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    data       TEXT    NOT NULL,
    builtin    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Transcrições por arquivo. `words` é JSON [{text,start,end}] em segundos da fonte.
CREATE TABLE IF NOT EXISTS transcripts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source_path TEXT    NOT NULL,
    provider    TEXT    NOT NULL,
    language    TEXT,
    status      TEXT    NOT NULL DEFAULT 'queued',
    error_msg   TEXT,
    words       TEXT    NOT NULL DEFAULT '[]',
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_transcripts_source ON transcripts(source_path);
