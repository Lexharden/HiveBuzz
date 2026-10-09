-- Configuración (colores, fuentes, posición, comportamiento) de cada overlay, como JSON.
-- Solo guarda lo que el usuario cambió; los valores por defecto viven en el código.
CREATE TABLE overlay_config (
    id   TEXT PRIMARY KEY NOT NULL,
    json TEXT NOT NULL
);
