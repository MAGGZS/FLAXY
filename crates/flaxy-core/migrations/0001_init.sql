CREATE TABLE IF NOT EXISTS templates (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    name               TEXT    NOT NULL,
    file_path          TEXT    NOT NULL,
    overlay_x          INTEGER NOT NULL DEFAULT 0,
    overlay_y          INTEGER NOT NULL DEFAULT 0,
    overlay_w          INTEGER NOT NULL DEFAULT 1080,
    overlay_h          INTEGER NOT NULL DEFAULT 1920,
    fit_mode           TEXT    NOT NULL DEFAULT 'cover',
    output_w           INTEGER NOT NULL DEFAULT 1080,
    output_h           INTEGER NOT NULL DEFAULT 1920,
    output_format      TEXT    NOT NULL DEFAULT 'mp4',
    video_bitrate      TEXT    NOT NULL DEFAULT '8M',
    audio_source       TEXT    NOT NULL DEFAULT 'raw',
    audio_mix_raw      REAL    NOT NULL DEFAULT 1.0,
    audio_mix_template REAL    NOT NULL DEFAULT 0.5,
    duration_rule      TEXT    NOT NULL DEFAULT 'raw',
    created_at         TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'queued',
    canceled    INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS job_items (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id      INTEGER NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    input_path  TEXT    NOT NULL,
    overrides   TEXT,
    output_path TEXT,
    status      TEXT    NOT NULL DEFAULT 'queued',
    progress    INTEGER NOT NULL DEFAULT 0,
    error_msg   TEXT,
    log_path    TEXT
);

CREATE INDEX IF NOT EXISTS idx_job_items_job ON job_items(job_id);
CREATE INDEX IF NOT EXISTS idx_job_items_status ON job_items(status);
