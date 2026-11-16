-- smooth_secs: duração (s) do deslize entre áreas quando smooth = 1
ALTER TABLE clip_items ADD COLUMN smooth_secs REAL NOT NULL DEFAULT 0.6;
