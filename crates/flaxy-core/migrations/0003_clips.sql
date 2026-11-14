-- Projetos de clipes de retenção: um vídeo longo (link) -> vários clipes curtos.
CREATE TABLE IF NOT EXISTS clip_projects (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    url           TEXT    NOT NULL,
    title         TEXT    NOT NULL DEFAULT '',
    source_path   TEXT,
    duration      REAL    NOT NULL DEFAULT 0,
    width         INTEGER NOT NULL DEFAULT 0,
    height        INTEGER NOT NULL DEFAULT 0,
    -- downloading | transcribing | analyzing | ready | error
    status        TEXT    NOT NULL DEFAULT 'downloading',
    progress      INTEGER NOT NULL DEFAULT 0,
    error_msg     TEXT,
    transcript_id INTEGER,
    min_len       REAL    NOT NULL DEFAULT 30,
    max_len       REAL    NOT NULL DEFAULT 60,
    layout        TEXT    NOT NULL DEFAULT 'vertical_camera',
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Sugestões da IA. Só as aprovadas são renderizadas.
CREATE TABLE IF NOT EXISTS clip_items (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id       INTEGER NOT NULL REFERENCES clip_projects(id) ON DELETE CASCADE,
    start            REAL    NOT NULL,
    end              REAL    NOT NULL,
    score            INTEGER NOT NULL DEFAULT 0,
    title            TEXT    NOT NULL DEFAULT '',
    reason           TEXT    NOT NULL DEFAULT '',
    approved         INTEGER NOT NULL DEFAULT 0,
    layout           TEXT    NOT NULL DEFAULT 'vertical_camera',
    camera_rect      TEXT,
    screen_rect      TEXT,
    caption_style_id INTEGER,
    -- none | queued | processing | done | error
    status           TEXT    NOT NULL DEFAULT 'none',
    progress         INTEGER NOT NULL DEFAULT 0,
    output_path      TEXT,
    error_msg        TEXT
);

CREATE INDEX IF NOT EXISTS idx_clip_items_project ON clip_items(project_id);
