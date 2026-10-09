-- Biblioteca local de sonidos. El archivo vive en <datos>/sounds/<file>.
CREATE TABLE sounds (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    file       TEXT    NOT NULL,
    volume     INTEGER NOT NULL DEFAULT 100, -- 0-100
    created_ms INTEGER NOT NULL
);
