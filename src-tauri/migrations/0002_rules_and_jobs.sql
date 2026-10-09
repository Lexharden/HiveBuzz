-- Reglas (trigger → condiciones → acciones), guardadas como JSON.
CREATE TABLE rules (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    json       TEXT    NOT NULL,
    updated_ms INTEGER NOT NULL
);

-- Jobs de la cola de acciones aún pendientes, para no perderlos si la app se cierra.
CREATE TABLE action_jobs (
    id         TEXT    PRIMARY KEY NOT NULL,
    expires_ms INTEGER NOT NULL,
    next_step  INTEGER NOT NULL DEFAULT 0,
    payload    TEXT    NOT NULL
);

CREATE INDEX idx_action_jobs_expires ON action_jobs (expires_ms);
