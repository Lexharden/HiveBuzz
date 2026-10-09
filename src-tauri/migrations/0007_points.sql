-- Base de datos de espectadores: puntos, estadísticas y primera/última vez visto.
-- `user_id` es el id numérico de TikTok. Un espectador importado por CSV sin id conocido se guarda
-- con un id provisional `unique:<usuario>` y se une a su id real la primera vez que aparece.
CREATE TABLE viewers (
    user_id       TEXT    PRIMARY KEY NOT NULL,
    unique_id     TEXT    NOT NULL,
    nickname      TEXT    NOT NULL,
    avatar        TEXT    NOT NULL DEFAULT '',
    points        INTEGER NOT NULL DEFAULT 0,
    total_earned  INTEGER NOT NULL DEFAULT 0,
    total_spent   INTEGER NOT NULL DEFAULT 0,
    coins_gifted  INTEGER NOT NULL DEFAULT 0,
    comments      INTEGER NOT NULL DEFAULT 0,
    likes         INTEGER NOT NULL DEFAULT 0,
    shares        INTEGER NOT NULL DEFAULT 0,
    watch_minutes INTEGER NOT NULL DEFAULT 0,
    first_seen_ms INTEGER NOT NULL,
    last_seen_ms  INTEGER NOT NULL
);

CREATE INDEX idx_viewers_points ON viewers (points DESC);
CREATE INDEX idx_viewers_unique ON viewers (unique_id);

-- Historial de movimientos de puntos (se rota: 90 días).
CREATE TABLE point_history (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    ts      INTEGER NOT NULL,
    user_id TEXT    NOT NULL,
    delta   INTEGER NOT NULL,
    reason  TEXT    NOT NULL
);

CREATE INDEX idx_point_history_user ON point_history (user_id, ts);
CREATE INDEX idx_point_history_ts ON point_history (ts);
