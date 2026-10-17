-- Último nivel conocido del espectador en TikTok: club de fans (`team_level`) y de regalos
-- (`gifter_level`). 0 = desconocido: TikTok no lo envía en todos los eventos, así que un evento sin
-- nivel nunca borra el que ya se conocía.
ALTER TABLE viewers ADD COLUMN team_level INTEGER NOT NULL DEFAULT 0;
ALTER TABLE viewers ADD COLUMN gifter_level INTEGER NOT NULL DEFAULT 0;
