# Seguridad

HiveBuzz corre en tu propio equipo: el servidor local solo escucha en `127.0.0.1`, los overlays y la API piden un token, y los secretos
(sesión de TikTok, tokens de Twitch y Spotify, contraseña de OBS) se guardan en el llavero del sistema.

## Cómo avisar de una vulnerabilidad
**No abras un issue público.** Usa *Security → Report a vulnerability* en la página del repositorio (aviso privado de GitHub).
Incluye qué pasa, cómo repetirlo y qué versión usas. Responderemos lo antes posible.

## Qué NO compartir nunca (ni en issues ni en capturas)
- Tu archivo `.env`, la clave privada de firma (`*.key`) ni las URLs de tus overlays (llevan tu token).
- Registros sin revisar: pueden incluir nombres de usuario.
