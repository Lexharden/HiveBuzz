// Núcleo común de los overlays de HiveBuzz. Se inyecta en cada página (no es un archivo aparte)
// para que cada overlay siga siendo una página independiente con su propia URL.
//
// Reglas de seguridad: todo texto que viene de TikTok se inserta con `textContent`, nunca como HTML;
// las imágenes remotas solo se aceptan por HTTPS.
window.HB = (function () {
  "use strict";

  var params = new URLSearchParams(location.search);
  var token = params.get("token") || "";
  var preview = params.get("preview") === "1";

  // Textos de los overlays. El idioma se elige con `?lang=` (por defecto español); añadir otro idioma
  // es añadir otro diccionario aquí.
  var dictionaries = {
    es: {
      "ev.gift": "envió {count}× {name}",
      "ev.follow": "empezó a seguirte",
      "ev.share": "compartió el LIVE",
      "ev.subscribe": "se suscribió",
      "ev.emote": "envió un emote",
      "ev.join": "entró al LIVE",
      "ev.like": "dio {count} likes",
      "ev.liveEnd": "El LIVE terminó",
      "coins": "monedas",
      "bits": "bits",
      "mod": "Moderador",
      "sub": "Suscriptor",
      "np.playing": "Sonando ahora",
      "np.paused": "En pausa",
      "np.requestedBy": "Pedida por {user}",
      "poll.votes": "{n} votos",
      "poll.ended": "Finalizada"
    },
    en: {
      "ev.gift": "sent {count}× {name}",
      "ev.follow": "started following you",
      "ev.share": "shared the LIVE",
      "ev.subscribe": "subscribed",
      "ev.emote": "sent an emote",
      "ev.join": "joined the LIVE",
      "ev.like": "sent {count} likes",
      "ev.liveEnd": "The LIVE has ended",
      "coins": "coins",
      "bits": "bits",
      "mod": "Moderator",
      "sub": "Subscriber",
      "np.playing": "Now playing",
      "np.paused": "Paused",
      "np.requestedBy": "Requested by {user}",
      "poll.votes": "{n} votes",
      "poll.ended": "Finished"
    }
  };
  var lang = dictionaries[params.get("lang")] ? params.get("lang") : "es";

  function t(key, vars) {
    var s = (dictionaries[lang] && dictionaries[lang][key]) || dictionaries.es[key] || key;
    if (vars) Object.keys(vars).forEach(function (k) { s = s.split("{" + k + "}").join(String(vars[k])); });
    return s;
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = String(text);
    return e;
  }

  /** Nombre de la moneda del evento: bits en Twitch, monedas en TikTok. */
  function coinsLabel(ev) { return t(ev && ev.platform === "twitch" ? "bits" : "coins"); }

  /** Insignia con la plataforma de origen (texto fijo con textContent; la clase sale de una lista cerrada). */
  function platformBadge(ev) {
    var p = ev && ev.platform === "twitch" ? "twitch" : "tiktok";
    return el("span", "hb-platform " + p, p === "twitch" ? "Twitch" : "TikTok");
  }

  function isHttps(u) { return typeof u === "string" && /^https:\/\//.test(u); }
  function isLocalMedia(u) { return typeof u === "string" && /^\/media\/[A-Za-z0-9._-]+$/.test(u); }
  function fmt(n) { return new Intl.NumberFormat(lang).format(Number(n) || 0); }
  function clamp(n, lo, hi) { return Math.min(Math.max(n, lo), hi); }

  /** Imagen remota segura (HTTPS, sin referrer). Devuelve null si la URL no es válida. */
  function img(url, cls) {
    if (!isHttps(url)) return null;
    var i = el("img", cls);
    i.referrerPolicy = "no-referrer";
    i.alt = "";
    i.src = url;
    i.onerror = function () { i.remove(); };
    return i;
  }

  /** Color estable por usuario (el mismo usuario siempre tiene el mismo tono). */
  function userColor(id) {
    var h = 0, s = String(id || "");
    for (var k = 0; k < s.length; k++) h = (h * 31 + s.charCodeAt(k)) >>> 0;
    return "hsl(" + (h % 360) + ", 70%, 65%)";
  }

  function hexToRgba(hex, alphaPct) {
    var m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex || "");
    if (!m) return "rgba(20,20,24,0.82)";
    return "rgba(" + parseInt(m[1], 16) + "," + parseInt(m[2], 16) + "," + parseInt(m[3], 16) + "," + clamp(alphaPct, 0, 100) / 100 + ")";
  }

  /** Traduce la configuración validada a variables CSS (estilo común de todos los overlays). */
  function applyStyle(cfg) {
    var r = document.documentElement.style;
    if (cfg.fontFamily) r.setProperty("--hb-font", '"' + cfg.fontFamily + '", system-ui, sans-serif');
    if (cfg.fontSize) r.setProperty("--hb-size", cfg.fontSize + "px");
    if (cfg.textColor) r.setProperty("--hb-fg", cfg.textColor);
    if (cfg.accentColor) r.setProperty("--hb-accent", cfg.accentColor);
    if (cfg.backgroundColor) r.setProperty("--hb-bg", hexToRgba(cfg.backgroundColor, cfg.backgroundOpacity));
    if (cfg.borderRadius !== undefined) r.setProperty("--hb-radius", cfg.borderRadius + "px");
    if (cfg.margin !== undefined) r.setProperty("--hb-margin", cfg.margin + "px");
    if (cfg.scale) r.setProperty("--hb-scale", String(cfg.scale / 100));
    if (cfg.anchor) document.body.setAttribute("data-anchor", cfg.anchor);
  }

  /** Quita los elementos más viejos para no pasar de `max`. */
  function trim(container, max) {
    while (container.children.length > max) {
      var oldest = container.firstChild;
      if (!oldest) break;
      container.removeChild(oldest);
    }
  }

  /** Programa el desvanecimiento y la retirada de un elemento (0 = no desaparece). */
  function expireAfter(node, seconds) {
    if (!seconds || seconds <= 0) return;
    setTimeout(function () {
      node.classList.add("hb-out");
      setTimeout(function () { node.remove(); }, 700);
    }, seconds * 1000);
  }

  /**
   * Conecta con HiveBuzz y entrega: configuración (`onConfig`), eventos (`onEvent(ev, replay)`) y
   * mensajes de otros canales (`onOverlay(channel, data)`). Reconecta solo con espera creciente.
   */
  function start(opts) {
    var root = document.getElementById("hb-root");
    if (preview) document.body.classList.add("hb-preview");
    var dot = document.getElementById("hb-dot");
    var delay = 1000;
    var configChannel = "config:" + opts.id;
    var firstConfig = true;
    // El servidor reenvía el historial en cada conexión: lo ya mostrado no se repite al reconectar.
    var seen = {};
    var seenOrder = [];
    function deliver(ev, replay) {
      if (!opts.onEvent || !ev) return;
      if (typeof ev.id === "string" && ev.id) {
        if (seen[ev.id]) return;
        seen[ev.id] = true;
        seenOrder.push(ev.id);
        if (seenOrder.length > 1000) delete seen[seenOrder.shift()];
      }
      opts.onEvent(ev, replay);
    }

    function connect() {
      var proto = location.protocol === "https:" ? "wss:" : "ws:";
      var ws = new WebSocket(proto + "//" + location.host + "/ws?token=" + encodeURIComponent(token));
      ws.onopen = function () { delay = 1000; if (dot) dot.classList.remove("show"); };
      ws.onmessage = function (m) {
        var msg;
        try { msg = JSON.parse(m.data); } catch (e) { return; }
        try {
          if (msg.type === "overlay") {
            if (msg.channel === configChannel && msg.data) {
              applyStyle(msg.data);
              if (opts.onConfig) opts.onConfig(msg.data, firstConfig);
              firstConfig = false;
            } else if (opts.onOverlay) {
              opts.onOverlay(msg.channel, msg.data);
            }
          } else if (msg.type === "history") {
            if (Array.isArray(msg.events)) msg.events.forEach(function (e) { deliver(e, true); });
          } else if (msg.type === "event") {
            deliver(msg.event, false);
          }
        } catch (e) { /* un mensaje raro no debe tumbar el overlay */ }
      };
      ws.onclose = function () {
        if (dot) dot.classList.add("show");
        setTimeout(connect, delay);
        delay = Math.min(delay * 2, 15000);
      };
      ws.onerror = function () { ws.close(); };
    }
    connect();
    return { root: root };
  }

  return {
    params: params, token: token, preview: preview,
    t: t, el: el, img: img, coinsLabel: coinsLabel, platformBadge: platformBadge, isHttps: isHttps, isLocalMedia: isLocalMedia,
    fmt: fmt, clamp: clamp, userColor: userColor, trim: trim, expireAfter: expireAfter,
    applyStyle: applyStyle, start: start
  };
})();
