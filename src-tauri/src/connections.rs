//! Todas las conexiones de la app (una por plataforma) vistas como un conjunto: sus estados para la UI y el
//! total de espectadores. Cada conexión publica sus eventos en el mismo bus, estampando su plataforma.

use std::sync::Arc;

use serde::Serialize;
use tokio::sync::watch;

use crate::connection::ConnectionService;
use crate::events::Platform;
use crate::source::StatusUpdate;

/// Estado de una plataforma, tal como lo recibe la UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformStatus {
    pub platform: Platform,
    #[serde(flatten)]
    pub status: StatusUpdate,
}

#[derive(Clone)]
pub struct ConnectionManager {
    tiktok: Arc<ConnectionService>,
    twitch: Arc<ConnectionService>,
}

impl ConnectionManager {
    pub fn new(tiktok: Arc<ConnectionService>, twitch: Arc<ConnectionService>) -> Self {
        Self { tiktok, twitch }
    }

    pub fn service(&self, platform: Platform) -> &Arc<ConnectionService> {
        match platform {
            Platform::Tiktok => &self.tiktok,
            Platform::Twitch => &self.twitch,
        }
    }

    pub fn statuses(&self) -> Vec<PlatformStatus> {
        Platform::ALL.iter().map(|p| PlatformStatus { platform: *p, status: self.service(*p).status() }).collect()
    }

    /// Un receptor por plataforma, para reenviar sus cambios de estado a la UI.
    pub fn subscribe_statuses(&self) -> Vec<(Platform, watch::Receiver<StatusUpdate>)> {
        Platform::ALL.iter().map(|p| (*p, self.service(*p).subscribe_status())).collect()
    }

    /// Espectadores sumados de todas las plataformas. Requiere un runtime de Tokio.
    pub fn spawn_total_viewers(&self) -> watch::Receiver<u64> {
        let (mut a, mut b) = (self.tiktok.subscribe_viewers(), self.twitch.subscribe_viewers());
        let (tx, rx) = watch::channel(*a.borrow() + *b.borrow());
        tokio::spawn(async move {
            loop {
                let alive = tokio::select! {
                    r = a.changed() => r.is_ok(),
                    r = b.changed() => r.is_ok(),
                };
                if !alive {
                    break;
                }
                let total = *a.borrow_and_update() + *b.borrow_and_update();
                tx.send_replace(total);
            }
        });
        rx
    }
}

#[cfg(test)]
mod tests;
