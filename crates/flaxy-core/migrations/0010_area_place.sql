-- Posição e tamanho das áreas encaixadas dentro do espaço delas no clipe
-- (JSON {"camera":{"x","y","scale"},"screen":{...}}; NULL = centro, inteiro).
ALTER TABLE clip_items ADD COLUMN area_place TEXT;
