-- Deslize e fundo dos clipes:
-- smooth_curve: curva de velocidade do deslize, JSON {x1,y1,x2,y2} (NULL = suave padrão)
-- bg_mode: fundo das áreas em Encaixar: blur | soft | black | file
-- bg_path: arquivo de fundo quando bg_mode = file
ALTER TABLE clip_items ADD COLUMN smooth_curve TEXT;
ALTER TABLE clip_items ADD COLUMN bg_mode TEXT NOT NULL DEFAULT 'blur';
ALTER TABLE clip_items ADD COLUMN bg_path TEXT;

-- Biblioteca de fundos personalizados para os clipes (aba Modelos).
CREATE TABLE IF NOT EXISTS clip_backgrounds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
