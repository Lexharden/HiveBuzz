//! Chatbot: comandos personalizados, respuestas por palabra clave, mensajes temporizados,
//! agradecimientos automáticos y los comandos de puntos. Escribe en el chat de TikTok a través de
//! la sesión del usuario (opcional: sin ella todo funciona en modo solo lectura).

pub mod engine;
pub mod model;
pub mod outbox;
pub mod service;
pub mod timed;
