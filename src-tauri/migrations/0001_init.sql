-- Ajustes generales (clave/valor). Los secretos NO van aquí: viven en el llavero del sistema.
CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- Registro de eventos normalizados. Se rota: se borra lo anterior a 7 días.
CREATE TABLE event_log (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT    NOT NULL,
    type     TEXT    NOT NULL,
    ts       INTEGER NOT NULL, -- ms desde epoch
    payload  TEXT    NOT NULL  -- LiveEvent en JSON
);

CREATE INDEX idx_event_log_ts ON event_log (ts);
