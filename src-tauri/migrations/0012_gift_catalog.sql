-- Regalos aprendidos en los directos (nombres en otro idioma, alias, precio, veces vistos) y altas manuales.
-- La lista oficial (resources/gifts/gifts.json) va con la app y no se guarda aquí: solo lo que la completa.
CREATE TABLE gift_catalog (
    key       TEXT    PRIMARY KEY,        -- el id del regalo en texto, o `name:<clave del nombre>` si no hay id
    gift_id   INTEGER,
    name_en   TEXT,
    name_es   TEXT,
    also      TEXT    NOT NULL DEFAULT '[]', -- JSON: otros nombres con los que llegó
    diamonds  INTEGER,                   -- precio por unidad
    seen      INTEGER NOT NULL DEFAULT 0,
    last_seen INTEGER,                   -- ms desde epoch
    manual    INTEGER NOT NULL DEFAULT 0  -- 1 = alta o edición manual
);
