-- Metas (likes, follows, monedas, regalo concreto…), como JSON; incluye su progreso actual.
CREATE TABLE goals (
    id       TEXT    PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    json     TEXT    NOT NULL
);

-- Timers/subathon: configuración y estado (corriendo, pausado, hora de fin), como JSON.
CREATE TABLE timers (
    id       TEXT    PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    json     TEXT    NOT NULL
);

-- Monedas regaladas por usuario y día. De aquí salen los rankings «del día» e «histórico».
CREATE TABLE donor_daily (
    day       TEXT    NOT NULL, -- YYYY-MM-DD (hora local)
    user_id   TEXT    NOT NULL,
    unique_id TEXT    NOT NULL,
    nickname  TEXT    NOT NULL,
    avatar    TEXT    NOT NULL,
    coins     INTEGER NOT NULL DEFAULT 0,
    gifts     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, user_id)
);

CREATE INDEX idx_donor_daily_user ON donor_daily (user_id);
