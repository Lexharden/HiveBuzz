-- Biblioteca local de imágenes, GIF y videos para las alertas. El archivo vive en <datos>/media/<file>.
CREATE TABLE media (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    file       TEXT    NOT NULL,
    kind       TEXT    NOT NULL, -- 'image' | 'video'
    created_ms INTEGER NOT NULL
);
