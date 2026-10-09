-- Perfiles: instantáneas con nombre de las reglas y la configuración de los overlays
-- («Minecraft», «Charla», «Sleep stream»…) que se aplican de un golpe.
CREATE TABLE profiles (
    id         TEXT    PRIMARY KEY NOT NULL,
    name       TEXT    NOT NULL,
    updated_ms INTEGER NOT NULL,
    json       TEXT    NOT NULL
);
