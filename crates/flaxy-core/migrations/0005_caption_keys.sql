-- caption_keys: trocas de posição/tamanho da legenda ao longo do clipe,
-- JSON [{t, adjust: {position, margin, size}}] com t em segundos da fonte.
ALTER TABLE clip_items ADD COLUMN caption_keys TEXT;
