-- Resumen por transmisión (una fila por sesión de LIVE). Las columnas sueltas sirven para listar;
-- el detalle (regalos por tipo, top donadores) va en `json`.
CREATE TABLE stream_stats (
    id           INTEGER PRIMARY KEY NOT NULL, -- inicio de la sesión, ms desde epoch
    ended_ms     INTEGER NOT NULL,             -- última actividad
    coins        INTEGER NOT NULL DEFAULT 0,
    peak_viewers INTEGER NOT NULL DEFAULT 0,
    json         TEXT    NOT NULL
);
