-- Likes ("tap taps") dados por usuario y día. De aquí sale el ranking «del día» del overlay de Top likes.
CREATE TABLE like_daily (
    day       TEXT    NOT NULL, -- YYYY-MM-DD (hora local)
    user_id   TEXT    NOT NULL,
    unique_id TEXT    NOT NULL,
    nickname  TEXT    NOT NULL,
    avatar    TEXT    NOT NULL,
    likes     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, user_id)
);
