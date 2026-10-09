//! Sistema de puntos: los espectadores ganan puntos por ver, comentar, dar likes, compartir,
//! seguir, suscribirse y regalar; los gastan en recompensas. Base de datos de espectadores con
//! historial e importación/exportación CSV.

pub mod config;
pub mod csv_io;
pub mod logic;
pub mod service;

use serde::Serialize;

/// Un espectador tal como se guarda y se muestra.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewer {
    pub user_id: String,
    pub unique_id: String,
    pub nickname: String,
    pub avatar: String,
    pub points: u64,
    pub total_earned: u64,
    pub total_spent: u64,
    pub coins_gifted: u64,
    pub comments: u64,
    pub likes: u64,
    pub shares: u64,
    pub watch_minutes: u64,
    pub first_seen_ms: i64,
    pub last_seen_ms: i64,
}

/// Movimiento del historial de puntos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub ts: i64,
    pub delta: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Points,
    Name,
    LastSeen,
    CoinsGifted,
}

/// Prefijo del id provisional de un espectador importado cuyo id de TikTok aún no se conoce.
pub const PROVISIONAL_PREFIX: &str = "unique:";
