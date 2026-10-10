# Guía de HiveBuzz – paso a paso

HiveBuzz es una app de escritorio **gratis** para streamers de TikTok LIVE. Todo corre en tu PC: no hay servidor
de por medio. Se conecta a tu LIVE, convierte lo que pasa (regalos, chat, follows…) en **eventos** y tú decides
qué hacer con ellos: sonidos, alertas en pantalla, voz, bot, puntos, OBS, etc.

> Estado honesto: está todo programado y probado con simulaciones, pero **no se ha probado con un LIVE real, Spotify
> real ni OBS real**. Si algo falla en vivo, mira la pestaña *Ajustes → Registro*.

---

## 1. Cómo funciona (en 30 segundos)

```
TikTok LIVE → HiveBuzz lee los eventos → REGLAS ("si pasa X, haz Y") → ACCIONES
                                      ↘ overlays (OBS), puntos, metas, estadísticas
```

- **Regla** = *cuándo* (regalo, follow, comando de chat…) + *condiciones* (cooldown, rol, probabilidad…) + *acciones*.
- **Acción** = algo que se ejecuta: sonido, alerta, voz, mensaje del bot, tecla, OBS, webhook…
- **Overlay** = una página web que pegas en OBS como *Browser Source*; muestra alertas, chat, metas, etc.
- **Simulador** (pestaña Panel) genera eventos falsos: úsalo para probar todo sin estar en vivo.

Variables que puedes usar en los textos: `{user}` `{nickname}` `{gift}` `{count}` `{coins}` `{text}` `{args}` `{likes}`.

---

## 2. Arrancar

1. Instala requisitos (Rust, Bun, Node) y en la carpeta del proyecto: `bun install`.
2. Una vez: `bun run sidecar:build` (empaqueta el conector de TikTok).
3. Copia `.env.example` a `.env` y rellénalo si lo necesitas (todo es opcional). Comprueba con `bun run app:check`.
4. Desarrollo: `bun run app:dev`. Instalador final: `bun run app:build` (carga el `.env` solo).

---

## 2b. Cómo está organizada la app

Menú a la izquierda, agrupado por lo que quieres hacer: **Inicio** · **Automatización** (Reglas, Metas y timers) · **Comunidad** (Bot y puntos, Estadísticas) ·
**Pantalla y sonido** (Overlays, Biblioteca, Voz) · **Conectar con otros** (OBS, Spotify, API) · **La aplicación** (Ajustes). Abajo del menú siempre ves si TikTok y Twitch están conectados.
Cada pantalla trae una frase que explica para qué sirve.

**¿Perdido?** Entra en **Ayuda** (en el menú): ahí está el **recorrido interactivo** (te enseña la app probándola contigo), la explicación de cada parte del menú, un glosario y respuestas rápidas. La primera vez que abras la app te lo ofrece solo.

## 3. Primer uso: conectar a tu LIVE

1. Pestaña **Panel** → escribe tu `@usuario` de TikTok → **Conectar**.
2. Estados: *Desconectado → Esperando LIVE* (aún no abriste tu LIVE) *→ Conectado*. Si se cae, reconecta solo.
3. Verás los eventos pasar en el feed. Sin LIVE, usa el **Simulador** (botones de regalo, chat, follow…).
4. **Twitch:** en la tarjeta de Twitch escribe el nombre de tu canal y pulsa **Conectar**: ya se lee el chat, los bits y las suscripciones, sin iniciar sesión.
   Para ver también **seguidores nuevos**, saber si estás en directo y cuántos te ven, pulsa **Entrar con Twitch**: se abre twitch.tv/activate, escribes el código que ves en
   pantalla, aceptas y HiveBuzz lo detecta solo. (HiveBuzz no escribe en el chat de Twitch.)
5. **Primeros pasos:** en Inicio hay una lista que se va marcando sola (conectar, poner un overlay en OBS, probar una alerta, crear una regla).
6. (Opcional) **Ajustes → API key de Euler Stream**: mejora la firma. Sin ella funciona el nivel gratuito.

---

## 4. Overlays (lo que se ve en pantalla)

1. Pestaña **Overlays**: elige uno (Alertas, Feed, Chat, Regalos recientes, Top donadores, Metas, Timer,
   Contadores, Ruleta, Encuesta, Sonando ahora).
2. Cambia colores, fuente, posición, tamaño… la **vista previa** se actualiza al instante. Botón ▶ *Probar* manda ejemplos.
3. En **Ajustes → Overlays** copia la URL del overlay.
4. En OBS (o TikTok LIVE Studio): *Fuente → Navegador (Browser Source)* → pega la URL → ancho 1920, alto 1080.
5. ⚠️ La URL lleva tu token: **no la compartas**.

---

## 5. Reglas: hacer que pasen cosas

Pestaña **Reglas → Nueva regla**:

1. **Nombre** (ej. "Gracias por la rosa").
2. **Cuándo (disparador)**: regalo (por nombre/id/monedas mínimas), follow, share, suscripción, entrada al LIVE,
   likes (cada N), comando de chat (`!sonido`), palabra clave, meta alcanzada, timer terminado, o *llamada a la API local*.
3. **Condiciones**: cooldown global y por usuario, roles (mod/suscriptor/seguidor), nivel mínimo, probabilidad, horario.
4. **Acciones** (en secuencia o paralelo, con retardo): elige el tipo y rellena el formulario.
5. **Guardar** y pulsa **Probar** (ignora condiciones) o usa el Simulador.

Los regalos grandes se adelantan en la cola. La cola tiene límite y caducidad; sobrevive a un cierre de la app.

### Tipos de acción
| Acción | Para qué |
|---|---|
| Reproducir sonido | Sonidos de tu biblioteca (volumen por sonido; varios = al azar) |
| Alerta en overlay | Imagen/GIF/video + texto con variables |
| Texto a voz | Lee un texto con la voz elegida |
| Modificar meta / Controlar timer | Sumar progreso, extender el subathon… |
| Mensaje del bot | El bot escribe en tu chat |
| Sumar/restar puntos | Premiar o cobrar puntos |
| Girar la ruleta / Lanzar encuesta | Interacción con el chat |
| Webhook HTTP | Avisar a IFTTT, Home Assistant, Streamer.bot… |
| Comando TCP / WebSocket | Hablar con juegos o mods |
| Simular teclas | Pulsar teclas en un juego (solo Windows) |
| OBS Studio | Cambiar escena, mostrar/ocultar fuente, filtros, grabar |

---

## 6. Sonidos, imágenes y voz

- **Biblioteca**: importa sonidos (mp3, wav, ogg, flac) e imágenes/GIF/video (png, jpg, gif, webp, mp4, webm). Ponles
  nombre y volumen; el botón *Escuchar* los previsualiza.
- **Voz (TTS)**: instala **Piper** (offline, un clic) y una voz, o usa las voces de Windows / Edge-TTS.
  - *Lectura del chat*: lee comentarios con filtros (solo `!tts`, roles, donadores recientes, usuarios ignorados).
  - Filtros de groserías, enlaces, emojis, letras repetidas y límite de caracteres. Voz única, por rol o aleatoria por usuario.
  - Botón **Saltar** para cortar lo que está sonando.

---

## 7. Metas y timers (pestaña "Metas y timers")

- **Meta**: barra de progreso de likes, follows, shares, suscripciones, monedas o un regalo concreto. Al llegar:
  se queda completa, se reinicia o sube el objetivo. Dispara el disparador *Meta alcanzada*.
- **Timer / subathon**: cuenta atrás que los regalos/likes/follows extienden (con tope). Dispara *Timer terminado*.
- Una **sesión** nueva empieza sola al reconectar tras terminar un LIVE (se puede forzar a mano).

---

## 8. Chatbot, puntos, ruleta y encuestas (pestaña "Bot y puntos")

1. **Iniciar sesión en TikTok** (sección *Sesión de TikTok*): se abre una ventana de TikTok; inicias sesión tú mismo
   (la app solo guarda las cookies en el llavero del sistema). Luego vuelve a conectar al LIVE. Sin sesión todo
   funciona en solo lectura; solo el bot necesita escribir.
2. **Bot** (nace apagado): comandos con alias (`!discord`), respuestas por palabra clave, mensajes temporizados,
   agradecimientos automáticos, `!puntos` y `!top`.
3. **Puntos**: se ganan por ver, comentar, likes, shares, follows, suscripciones y monedas. Tabla de espectadores
   con edición e importación/exportación CSV.
4. **Recompensas**: crea una regla con disparador *comando de chat* y ponle **coste en puntos**; el espectador la
   "canjea" escribiendo el comando (si falla, se le devuelven los puntos).
5. **Ruleta**: define premios con pesos; se lanza con la acción *Girar la ruleta* o el botón de prueba.
6. **Encuesta**: pregunta + 2 a 8 opciones; el chat vota escribiendo el número. Overlay de resultados.

---

## 9. Integraciones (pestaña "Integraciones")

### OBS
1. En OBS: *Herramientas → Ajustes del servidor WebSocket → Activar* (OBS 28+).
2. En HiveBuzz: host `127.0.0.1`, puerto `4455`, contraseña → **Guardar** → **Probar conexión**.
3. En una regla añade la acción *OBS Studio*. Con "Revertir tras (ms)" una fuente se muestra unos segundos y se oculta sola.

### Simular teclas
- Escribe la combinación (`ctrl+shift+f5`) y en **Ventanas permitidas** el título o `.exe` del juego (`Minecraft`).
  Si la ventana activa no coincide, **no envía nada**. No dejes "cualquier ventana" salvo que sepas lo que haces.

### Webhook y comandos de red
- Webhook: URL, método, cuerpo JSON (los textos con variables se escapan solos).
- TCP/WebSocket: host+puerto o URL y un mensaje con variables.

### API local (para scripts, Streamer.bot, mods)
1. Crea una regla con disparador **Llamada a la API local** y ponle un nombre (`mi-accion`).
2. Desde otro programa:
   `curl -X POST "http://127.0.0.1:PUERTO/api/trigger" -H "Authorization: Bearer TU_TOKEN" -H "Content-Type: application/json" -d "{\"name\":\"mi-accion\",\"vars\":{\"user\":\"ana\"}}"`
   (la pestaña muestra el comando ya con tu puerto y token). Responde 202 si se encoló.
3. También hay un WebSocket con todos los eventos en vivo. Todo solo escucha en `127.0.0.1` y exige el token.

### Spotify (peticiones de canciones)
1. La primera vez, abre *Integraciones → Spotify → Configurar mi app de Spotify* y sigue los 5 pasos (crear una
   app gratis en developer.spotify.com, pegar la Redirect URI, añadir tu cuenta en *User Management* y copiar el
   Client ID). Después pulsa *🎵 Conectar con Spotify*, inicia sesión en el navegador y acepta: verás "✅ Conectado".
2. Tus espectadores escriben `!sr nombre o enlace de la canción` y se agrega a tu cola; `!song` dice qué suena.
   (Quién puede pedir, coste en puntos, máximo por persona, etc. están en "Opciones de las peticiones", con
   valores razonables ya puestos.)
3. Añade el overlay **Sonando ahora** a OBS.
4. Requisitos de Spotify: tener Spotify abierto y reproduciendo, y cuenta **Premium** (sin Premium no se puede encolar).
- Si aparece **"403 Forbidden"**, tu cuenta no está en *User Management* de la app de Spotify (o la cuenta dueña
  de la app no tiene Premium). Añádela y vuelve a conectar.

---

## 10. Estadísticas, perfiles, copias de seguridad

- **Estadísticas**: una fila por LIVE: monedas, pico de espectadores, regalos por tipo, top donadores…
- **Perfiles** (*Ajustes*): guarda tus reglas+overlays con un nombre ("Minecraft", "Charla") y cámbialos de un golpe.
- **Exportar/Importar**: un `.zip` con reglas, metas, timers, overlays, perfiles, ajustes, sonidos e imágenes.
  Importar se aplica **al reiniciar** y **sustituye** lo actual. No incluye contraseñas ni tokens, pero sí lo que
  pongas en reglas (p. ej. cabeceras de un webhook).

---

## 11. Ajustes de la app

- Idioma **Español / English** (UI y overlays), cerrar a la **bandeja**, iniciar con Windows (minimizado opcional).
- **Registro (logs)**: ver, filtrar por nivel y guardar en archivo; útil para reportar problemas.
- **Actualizaciones**: indica `usuario/repo` de GitHub. Para publicar versiones necesitas generar una clave
  (`bunx tauri signer generate`), poner la pública en `src-tauri/tauri.conf.json → plugins.updater.pubkey` y los
  secretos de GitHub (detalle en el README). Sin clave pública no se actualiza nada.

---

## 12. Seguridad en una línea

El servidor solo escucha en tu PC (`127.0.0.1`), los overlays/API piden token, y los secretos (Euler, sesión de
TikTok, contraseña de OBS, Spotify) van al **llavero del sistema**, nunca a la base de datos.

## 13. Si algo no funciona

| Síntoma | Qué mirar |
|---|---|
| No conecta | Estado en el Panel; *Ajustes → Registro*; prueba la API key de Euler |
| Overlay vacío en OBS | URL completa con token; servidor local sin error en Ajustes; puerto libre |
| El bot no escribe | ¿Está activado? ¿Hay sesión de TikTok? Reconecta al LIVE después de iniciar sesión |
| Spotify no encola | Premium, dispositivo activo, Redirect URI exacta, "Conectado" en verde |
| Spotify: 403 Forbidden | Añade tu cuenta en *Settings → User Management* de tu app de Spotify y reconecta |
| Las teclas no se envían | La ventana activa debe estar en la lista blanca; si el juego es "administrador", abre HiveBuzz igual |
| Voz no suena | Instala Piper y una voz en la pestaña Voz, o elige una voz de Windows |

Lo que **no** existe: Minecraft/RCON (descartado). Para mods con socket propio usa *Comando TCP/WebSocket*.
