-- Edição por clipe na tela de Clipes.
-- area_keys: trocas de área ao longo do clipe, JSON [{t, camera, screen}] com
--   t em segundos da FONTE. Antes da primeira troca valem camera_rect/screen_rect.
-- area_fit: bits de "encaixar" (mostrar a área inteira, sobra com o vídeo
--   desfocado) em vez de preencher cortando. 1 = câmera, 2 = tela.
-- caption_adjust: posição e tamanho da legenda só deste clipe, JSON
--   {position, margin, size}. Nulo = usa o que vem do modelo de legenda.
ALTER TABLE clip_items ADD COLUMN area_keys TEXT;
ALTER TABLE clip_items ADD COLUMN area_fit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE clip_items ADD COLUMN caption_adjust TEXT;
