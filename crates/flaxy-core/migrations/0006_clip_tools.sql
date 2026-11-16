-- Ferramentas do editor de clipe:
-- split: parte da altura da tela nos modos tela + câmera (0.2 .. 0.8)
-- word_edits: correções de palavras da legenda, JSON [{t, text}]
-- cuts: trechos cortados de dentro do clipe, JSON [{a, b}] em segundos da fonte
-- smooth: trocas de área deslizam em vez de corte seco
ALTER TABLE clip_items ADD COLUMN split REAL NOT NULL DEFAULT 0.6;
ALTER TABLE clip_items ADD COLUMN word_edits TEXT;
ALTER TABLE clip_items ADD COLUMN cuts TEXT;
ALTER TABLE clip_items ADD COLUMN smooth INTEGER NOT NULL DEFAULT 0;
