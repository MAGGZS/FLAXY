-- Marcas d'água: textos translúcidos em posições escolhidas (aba Modelos).
-- items: JSON [{text, x, y, size, opacity, color, font, bold, outline}]
CREATE TABLE IF NOT EXISTS watermarks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    items TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Marca d'água ligada ao clipe (NULL = sem marca).
ALTER TABLE clip_items ADD COLUMN watermark_id INTEGER;
