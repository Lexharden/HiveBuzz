# Changelog

Formato basado en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/).

## [Sin publicar]

### Añadido
- **Alertas automáticas**: el overlay de Alertas muestra solo los regalos (desde un mínimo de monedas), follows y
  suscripciones (y, si se activa, shares), sin crear reglas. Se configuran en *Overlays → Alertas*; las reglas con
  «Mostrar alerta» siguen funcionando aparte (si ya las usas para lo mismo, apaga la automática para no verla doble).
- **Más voces**: Edge-TTS pasa de 14 a 92 voces (todas las variantes del español y voces en inglés, portugués,
  francés, italiano, alemán, japonés y coreano); el catálogo de Piper pasa de 6 a 39 voces, con filtro por idioma;
  en Windows aparecen también las voces «OneCore» de los paquetes de idioma (p. ej. Microsoft Raúl).
- **Voces personalizadas**: importar un modelo de Piper propio (`.onnx` + `.onnx.json`) y borrar voces instaladas.
- **No hablar encima del streamer**: con el micrófono activado en *Voz (TTS)*, la lectura espera a que el streamer
  calle y, si empieza a hablar a mitad, se pausa y al terminar repite la palabra cortada (o el mensaje entero, o lo
  salta, según se elija). Sensibilidad ajustable con medidor en vivo; solo se mide el nivel, no se graba nada.
- **Plantillas de reglas**: 13 reglas habituales (agradecimientos por voz, bienvenida, saludos, !discord, !redes,
  !comandos, meta de likes, !di por puntos, !anuncio de moderador) que se añaden con un clic desde *Reglas → Plantillas*.
- **Reglas**: el editor muestra el coste en puntos (la regla se vuelve una recompensa canjeable).
- **Actualizaciones**: el repositorio oficial `Lexharden/HiveBuzz` viene configurado por defecto (antes había que escribirlo).

### Corregido
- **Micrófono («no hablar encima»)**: los micrófonos se muestran con su nombre completo de Windows
  («Micrófono (Yeti Nano)», no solo «Micrófono», que era igual para todos y podía abrir otro); el medidor funciona en
  cuanto se elige el micrófono, sin guardar; la sensibilidad cambia sin reabrirlo; botón para volver a buscar
  micrófonos y aviso si llega casi en silencio (silenciado o sin volumen de entrada).
- **TikTok**: el nivel de fan (Fans Club) y el de regalos se leen también de las insignias; en los mensajes de chat
  TikTok no manda `fansClub`/`payGrade`, así que el filtro «Nivel de equipo mínimo» del TTS y de las reglas descartaba a todos.
- **TikTok**: moderador, suscriptor y seguidor se deducen también de insignias y `followInfo` (likes, follows y entradas
  no traen `userIdentity`); un follow marca al usuario como seguidor; los valores por defecto del protocolo («0», «»)
  ya no mezclan a usuarios distintos bajo el id «0».
- **Sidecar**: dos conexiones seguidas ya no dejan dos supervisores; los errores de la librería se registran legibles.
- **Spotify**: el «403 Forbidden» se explica (cuenta no añadida en *User Management*); un refresh token inválido o un
  cambio de Client ID cierran la sesión; conectar ya no desactiva luego las peticiones al guardar.
- **Cola de acciones**: los canjes descartados sin ejecutarse (caducados o desplazados) devuelven los puntos; un job
  se guarda antes de poder ejecutarse, así que no se repite al reiniciar.
- **Twitch**: EventSub no pierde notificaciones al reconectarse; cancelar el inicio de sesión ya no se deshace.
- **Overlays**: el historial que se reenvía al reconectar ya no duplica mensajes en OBS.
- **App**: una segunda instancia muestra la ventana existente en vez de duplicar conexiones y acciones; actualizar en
  Windows guarda antes el estado; sin llavero del sistema la app arranca igualmente; «cerrar a la bandeja» solo
  afecta a la ventana principal.
- **Seguridad**: CSP estricta en la ventana principal; importar una configuración ya no puede elegir el programa de
  Piper ni un OBS remoto; las secuencias de teclas no pueden durar más de 60 s.
- **Releases**: la etiqueta debe coincidir con la versión; la clave pública del auto-update se inyecta desde la variable
  `TAURI_UPDATER_PUBKEY` (y el release falla si hay clave privada sin pública); un único borrador para todas las
  plataformas; `collect-artifacts` incluye la actualización de macOS y descarta instaladores de versiones anteriores;
  `set-version` también cambia `sidecar/package.json`.

### Cambiado
- **Spotify**: la interfaz guía para que cada streamer cree su propia app (las apps en modo desarrollo solo admiten
  cuentas añadidas a mano). El Client ID integrado queda como opción.
- **TTS**: el registro dice por qué no se leyó un mensaje del chat.

## [0.1.0] - 2026-10-09

Primera versión pública. Incluye todo lo descrito en las fases 1 a 7 de más abajo.

### Fase 7 – Twitch y rediseño de la interfaz

#### Añadido
- **Scripts de compilación** `scripts/build.cmd|ps1` (Windows) y `scripts/build.sh` (Linux/macOS): comprueban el entorno, instalan, compilan y reúnen los instaladores con SHA-256 en `release/<versión>/`.
- **Logo e iconos propios** (a partir de `public/hivebuzz.png`, con fondo transparente) para la ventana, la bandeja, los instaladores y el menú; **tercer color de marca** naranja #f94a20.
- **Recorrido interactivo** (17 pasos; 2 de ellos esperan a que la persona pruebe el simulador o abra Reglas) con invitación la primera vez, y pantalla **Ayuda** con la explicación
  de cada parte del menú, glosario y preguntas frecuentes (español e inglés, con tests de que no falta ninguna).
- **Twitch (solo lectura)**: chat, bits (como regalos «Bits») y suscripciones por IRC anónimo —sin cuenta—; con **inicio de sesión opcional por código de dispositivo**
  (sin redirecciones ni contraseñas) también seguidores, estado del directo y espectadores (EventSub + Helix). Reconexión con backoff, deduplicación por id, vigilancia de inactividad.
- **Varias conexiones a la vez**: `Platform` en los eventos, una `ConnectionService` por plataforma, `ConnectionManager` (estados por plataforma y espectadores sumados).
  Los ids de Twitch llevan el prefijo `tw:`: sin migración de base de datos.
- El bot, los agradecimientos, las confirmaciones de canje y las respuestas de `!sr` solo salen por TikTok; los eventos de Twitch sí alimentan reglas, TTS, alertas, puntos, metas y votos.
- Simulador con selector de plataforma; overlays con la opción *Mostrar de qué plataforma viene* y bits en Twitch.
- **Interfaz rediseñada**: menú lateral agrupado con resumen de conexiones, descripción en cada pantalla, Inicio con tarjetas de conexión en lenguaje llano,
  diálogo de inicio de sesión de Twitch, lista de primeros pasos que se marca sola, filtro del feed por plataforma y simulador plegado.
- Tests: 640+ en Rust (IRC, EventSub, Device Code, fuente con servidores simulados, manager, gating del bot), 95 de overlays/UI/i18n.

#### Pendiente / no verificado
- Probado solo con servidores de mentira, no con un canal de Twitch real. Hace falta registrar una app de Twitch (tipo «Público») y compilar con `HIVEBUZZ_TWITCH_CLIENT_ID` para el botón de inicio de sesión.
- No se leen raids, puntos de canal ni hype train; no se puede escribir en el chat de Twitch.

### Fase 6 – Extras

#### Añadido
- **Versión visible** en la cabecera, **colores de marca** (azul #00346e y amarillo #ffc113) en la interfaz, los overlays
  (acento por defecto) y la página de retorno de Spotify.
- **Distribución multiplataforma**: release en Windows, macOS (ARM e Intel) y Linux desde GitHub Actions, CI en las tres,
  metadatos de instalador y `scripts/set-version.mjs` para cambiar la versión en los tres archivos a la vez.
- **Spotify** (OAuth PKCE sin servidor propio): `!sr` con permisos por rol, coste en puntos con reembolso,
  límite por persona, cooldown, duración máxima y listas de bloqueo; `!song`; enlaces/URI de Spotify;
  overlay **«Sonando ahora»** (`/overlay/nowplaying`). Refresh token en el llavero.
- **Estadísticas por transmisión**: monedas, pico de espectadores, comentarios/likes/follows/shares/
  suscripciones, top de donadores y regalos por tipo; se guardan en SQLite (se conservan las últimas 400).
- **Perfiles**: instantáneas de reglas + configuración de overlays que se aplican de un golpe.
- **Exportar / importar** la configuración completa en un `.zip` (con sonidos e imágenes), validada contra
  rutas maliciosas, extensiones, tamaños y reglas inválidas; la importación se aplica al reiniciar.
- **Auto-actualización** con el updater de Tauri desde GitHub Releases (repositorio configurable, firma
  obligatoria) y flujo de CI `release.yml` para publicar releases firmados.
- **Bandeja del sistema** (mostrar / salir, cerrar a la bandeja), **inicio con el sistema** y **logs** de la
  app visibles en la UI (filtro por nivel, exportar a archivo).
- **Inglés**: toda la interfaz y los textos de los overlays (`?lang=en`); el idioma se elige en Ajustes.
- API local: la vuelta de OAuth `/spotify/callback` (única ruta sin token, protegida por `state`).
- Tests: 584 en Rust, 73 de overlays/i18n (incluye la paridad de claves es/en).

#### Cambiado
- Compilación con `.env`: plantilla `.env.example`, lanzador `scripts/with-env.mjs` (`bun run app:dev|app:build|app:check`)
  que carga el archivo y valida las variables; `.env` y `*.key` quedan fuera de Git.
- Spotify: ahora es un solo botón **Conectar con Spotify** (Client ID integrado al compilar con
  `HIVEBUZZ_SPOTIFY_CLIENT_ID`); el Client ID propio y el Redirect URI pasan a «Opciones avanzadas».

#### Corregido
- La exportación a CSV de espectadores (Fase 4) usaba el diálogo de guardar sin tener concedido el permiso
  `dialog:allow-save`; ahora está en las capacidades de la ventana.

#### Pendiente / no verificado
- Probado solo contra servidores simulados: no contra Spotify real (login, `!sr`, cola), ni contra OBS real.
- No se ha ejecutado el flujo de release de GitHub ni una actualización real; hace falta generar el par de
  claves y poner la pública en `tauri.conf.json` (ver README).
- La bandeja, el inicio con el sistema y la importación al reiniciar no se han probado de forma interactiva.
- Los mensajes de error del backend y las respuestas por defecto del bot siguen en español.

### Fase 5 – Integraciones

#### Añadido
- **Webhook HTTP** (`webhook`): GET/POST/PUT/PATCH/DELETE, cabeceras y cuerpo con variables; solo
  http(s), tiempo máximo de 10 s, variables de la URL codificadas y cuerpos JSON escapados.
- **Comando genérico por red**: `tcpSend` y `wsSend` (host, puerto o URL y plantilla de mensaje).
- **Simulación de teclas** (`pressKeys`, Windows): combinaciones y secuencias, con **lista blanca de
  ventana destino** comprobada antes de cada combinación; scancodes de hardware; siempre se sueltan las
  teclas, aunque algo falle.
- **OBS Studio** (`obs`, obs-websocket v5 con autenticación): cambiar escena, mostrar/ocultar fuente,
  activar/desactivar filtro (con reversión automática opcional), iniciar/detener grabación.
  Contraseña en el llavero.
- **API local**: `POST /api/trigger`, `GET /api/triggers`, `GET /api/status` (además del WebSocket
  de eventos). Nuevo disparador de reglas **«Llamada a la API local»**. Acepta `?token=` o
  `Authorization: Bearer`; cuerpo limitado a 16 KB y variables validadas.
- UI: pestaña **Integraciones**, formularios para todas las acciones nuevas y para las de la Fase 4
  (`botMessage`, `pointsAdjust`, `startPoll`), y el disparador de API en el editor de reglas.
- Tests: 520 en Rust (con servidores TCP/HTTP/WS/OBS de mentira en loopback).

#### Pendiente / no verificado
- Probado solo contra servidores simulados: no contra un OBS real, ni con teclas enviadas a un juego real.
- Minecraft/RCON no se implementa (decisión del proyecto). `pressKeys` no existe en macOS/Linux.
- Las cabeceras de un webhook se guardan con la regla en SQLite (no en el llavero): no pongas ahí
  secretos que no quieras en disco.

### Fase 4 – Interacción

#### Añadido
- **Chatbot** (`bot`): comandos personalizados con alias y varias respuestas, respuestas por palabra
  clave, mensajes temporizados (con mínimo de actividad en el chat), agradecimientos automáticos a
  regalos/follows/shares/suscripciones, `!puntos` y `!top`, y confirmación de canjes. Variables
  `{user}`, `{nickname}`, `{coins}`, `{gift}`, `{points}`, `{count}`…; cooldowns y roles reutilizan las
  condiciones de las reglas. Nace **apagado**. No responde a eventos del simulador.
- **Salida al chat** con cola propia: ritmo mínimo, descarte de repetidos y de mensajes caducados,
  detección del eco del propio bot y registro visible en la UI.
- **Sesión de TikTok opcional**: ventana de login propia; solo se guardan las cookies de sesión, en el
  llavero del sistema. Sin sesión todo sigue en modo solo lectura.
- **Sistema de puntos**: por ver, comentar, likes, shares, follows, suscripciones y monedas; base de
  espectadores con historial, edición de saldo e importación/exportación CSV (protegida contra
  inyección de fórmulas). **Recompensas** = reglas con coste en puntos (cobro atómico y reembolso).
- **Ruleta de premios** (sorteo con pesos en Rust, overlay `/overlay/wheel`) y **encuestas por chat**
  (un voto por persona, cierre automático, overlay `/overlay/poll`).
- Acciones nuevas: `botMessage`, `pointsAdjust`, `spinWheel`, `startPoll`.
- Pestaña **Bot y puntos** en la UI.

#### Pendiente / no verificado
- No probado en vivo: login de TikTok, envío real al chat y la interfaz gráfica.
- Los premios de la ruleta con acciones propias solo se editan por la API (la UI edita nombre,
  peso y color); las acciones nuevas aún no tienen formulario en el editor de reglas.

### Fase 3 – Overlays

#### Añadido
- **8 overlays**, cada uno con su URL: alertas, feed de eventos, **chat en pantalla**, **regalos
  recientes**, **top donadores** (sesión / día / histórico), **metas** con barra de progreso,
  **timer / subathon** y **contadores** de likes y espectadores.
- **Editor visual** con **vista previa en vivo** (el propio overlay en un iframe a 1920×1080): fuente,
  tamaños, colores, opacidad, posición, escala y comportamiento de cada overlay. Los cambios llegan
  al overlay por el mismo WebSocket, sin recargar. El formulario se genera a partir de un **esquema
  definido en Rust** (valores por defecto, rangos, validación y claves de i18n): una sola fuente de
  verdad para el editor, el backend y los overlays.
- **Estado retenido** en el `OverlayHub` y **historial reciente**: un overlay recién abierto (o
  refrescado en OBS) recibe al instante su configuración, el estado de metas/timer/ranking y los
  últimos eventos de chat y regalos.
- **Metas** (`goals`): likes, follows, shares, suscripciones, monedas o un regalo concreto; al
  alcanzarse pueden quedarse completas, reiniciarse conservando el sobrante o subir el objetivo, y
  disparan el trigger «Meta alcanzada». Persistentes.
- **Timers / subathon** (`timers`): cuentan hacia atrás y se extienden por monedas (cada N), likes
  (cada N), follows, shares, suscripciones o un regalo concreto, con **tope máximo**; iniciar,
  pausar, reanudar, reiniciar, sumar y restar. Sobreviven a un reinicio de la app y disparan «Timer
  terminado».
- **Ranking de donadores** (`leaderboard`) por sesión (memoria), día e histórico (SQLite).
- **Sesión de transmisión** (`session`): una sesión nueva empieza al reconectar tras un fin de LIVE
  (o a mano) y reinicia el ranking de la sesión, los contadores y las metas que lo piden.
- **Contadores en vivo**: el sidecar emite ahora `viewers` (espectadores), con pico de la sesión.
- Acciones internas **`goalAdjust`** y **`timerControl`** (admiten variables numéricas como `{coins}`).
- Núcleo común de overlays (`overlays/common.js` y `.css`) con textos traducibles (`HB.t`, `?lang=`).
- Pestañas **Overlays** y **Metas y timers**; los disparadores «meta/timer» ahora se eligen de una lista.
- Tests: 349 en Rust, 54 en el sidecar y **46 de los propios overlays** ejecutados en jsdom (incluidos
  los de seguridad: apodos y textos con HTML, URLs `javascript:`/`http:`, rutas de medios).

#### Corregido
- `bun run sidecar:build` (y `tauri build`, que lo usa) no funcionaba desde la raíz por una sintaxis de
  Bun inválida (`--cwd`) heredada de la Fase 1.

#### Conocido
- El inglés de los textos de los overlays (`HB.t`) se completa en la Fase 6.
- El historial de chat/regalos de un overlay recién abierto viene de memoria (últimos 80 eventos).
- Las pruebas de la vista previa (iframe) y del editor no se han hecho a mano en un navegador real.

### Fase 2 – Núcleo de acciones

#### Añadido
- **Motor de reglas** declarativo (JSON): trigger → condiciones → acciones. Triggers: regalo
  (por id, nombre y/o monedas mínimas), follow, share, suscripción, entrada, emote de suscriptor,
  likes (cada N), comando de chat, palabra clave, meta alcanzada y timer terminado (estos dos los
  disparan las fases siguientes). Condiciones: cooldown global y por usuario, roles, niveles de
  equipo/donador, probabilidad y horario (puede cruzar la medianoche).
- **Cola de acciones** con prioridad (los regalos grandes se adelantan), concurrencia por tipo
  (un TTS a la vez, sonidos en paralelo, alertas de una en una), límite de tamaño con desplazamiento
  de lo menos prioritario, caducidad, tiempo máximo por acción y **persistencia en SQLite**: los
  jobs pendientes se recuperan al reabrir la app.
- Planes de acciones **en secuencia o en paralelo**, con retardos por acción.
- Trait `ActionExecutor` + `ExecutorRegistry`: agregar un ejecutor no toca el núcleo.
- Ejecutores: **`playSound`** (biblioteca local, volumen por sonido, varios al azar),
  **`overlayAlert`** (imagen/GIF/video + texto con variables) y **`tts`**.
- **Biblioteca de sonidos y de medios** (importar, previsualizar, volumen, renombrar, borrar).
  Los medios se sirven por `/media/<archivo>?token=…`; sin SVG a propósito.
- **Overlay de alertas** `/overlay/alerts` (cola en el cliente, posición `?pos=top|center|bottom`,
  reconexión automática) y canal genérico `OverlayHub` por el WebSocket local.
- **TTS**: motores Piper (offline), SAPI de Windows y Edge-TTS (opcional); filtros de groserías
  (editables, con modo omitir/censurar), enlaces, emojis, letras repetidas y límite de caracteres;
  lectura del chat con filtros (solo comando `!tts`, roles, niveles, donadores recientes,
  cooldown, usuarios ignorados); voz única, por rol o aleatoria estable por usuario; botón de saltar.
- **Instalador de Piper y sus voces** desde las fuentes oficiales (catálogo cerrado, solo HTTPS,
  SHA-256 fijado para Piper, extracción protegida contra zip-slip).
- Botón **Probar** en cada regla (evento de ejemplo que cumple el trigger, ignora condiciones).
- UI: pestañas Reglas (editor completo), Biblioteca y Voz (TTS).
- Tests: 227 en Rust (+2 de red marcados `#[ignore]`) y 51 en el sidecar.

#### Verificado a mano
- SAPI sintetiza un WAV real con el texto de un mensaje con comillas y `$()` (sin inyección).
- Edge-TTS: respuesta real con audio MP3 decodificable (la versión de Chromium del cliente es una
  constante que Microsoft puede dejar obsoleta: `tts/edge.rs`).
- Piper: instalación completa (zip verificado, voz `es_MX-claude-high`) y síntesis real.

#### Conocido
- La interfaz gráfica no se ha probado de forma interactiva en esta fase (compila, tipa y empaqueta).
- Los jobs persistidos se reanudan con semántica «al menos una vez»: si la app se cierra a media
  acción, esa acción puede repetirse al reabrir.
- La instalación automática de Piper solo existe en Windows; en macOS/Linux se configura la ruta.

### Fase 1 – Base

#### Añadido
- Proyecto Tauri 2 + React + Vite + TypeScript (strict) + Tailwind, con i18n en español.
- **Sidecar** Node/TypeScript con `tiktok-live-connector`, empaquetado con Bun como `externalBin`
  y conectado a Rust por NDJSON.
- **Normalizador** con deduplicación por `msgId` (LRU de 10 000), rachas de regalos combinables
  (solo con `repeatEnd`, conteo final), regalos grandes no combinables al primer mensaje, likes
  agregados en ventanas de 1 s y registro de mensajes desconocidos o malformados.
- Red de seguridad para rachas: si no llega `repeatEnd` en 10 s, se emite el último conteo visto
  (así un cierre perdido no pierde el regalo).
- **Resiliencia**: espera al LIVE, backoff exponencial con jitter (1 s → 60 s), heartbeat (45 s),
  reintento respetando el `retry-after` de la firma, y reinicio automático del sidecar con
  restauración de la conexión pedida.
- Estados visibles en la UI: desconectado / esperando LIVE / conectado / error de firma / reconectando.
- Trait `LiveSource` y bus de eventos (`tokio::broadcast`).
- **SQLite** (`sqlx`, migraciones) con ajustes y log de eventos con rotación a 7 días.
- **Llavero del sistema** para la API key de Euler y el token de overlays.
- **Servidor local** `axum` en `127.0.0.1` (puerto configurable) con token, comprobación de
  `Host`/`Origin`, WebSocket de eventos y overlay de prueba `/overlay/feed`.
- Dashboard: conectar/desconectar por `@usuario`, feed de eventos en vivo, ajustes (URLs de
  overlays, API key de Euler, puerto) y **simulador** de eventos.
- Tests: 51 del sidecar (normalizador, supervisor, protocolo) y 47 de Rust (protocolo, supervisor
  del sidecar, bus, SQLite, servidor, simulador, conexión).

#### Conocido
- El puerto del servidor local se aplica al reiniciar la app.
- No verificado contra un LIVE real en esta fase (sin cuenta en directo disponible durante el
  desarrollo); el flujo se probó con clientes simulados y con el binario real hasta `waiting_live`.
