//! Biblioteca local de imágenes, GIF y videos para las alertas del overlay.
//! Los archivos se sirven por el servidor local en `/media/<file>` (con token).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::actions::clock::Clock;
use crate::db::Db;
use crate::error::{AppError, Result};

/// Sin SVG a propósito: un SVG servido desde el propio origen podría ejecutar scripts.
pub(crate) const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];
pub(crate) const VIDEO_EXT: &[&str] = &["mp4", "webm"];
const MAX_BYTES: u64 = 60 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub id: String,
    pub name: String,
    /// Nombre del archivo en la carpeta de medios (`<uuid>.<ext>`).
    pub file: String,
    pub kind: MediaKind,
    pub created_ms: i64,
}

impl Media {
    /// Ruta relativa con la que el overlay lo pide al servidor local.
    pub fn url_path(&self) -> String {
        format!("/media/{}", self.file)
    }
}

pub struct MediaLibrary {
    db: Db,
    dir: PathBuf,
    clock: Arc<dyn Clock>,
}

impl MediaLibrary {
    pub fn new(db: Db, dir: PathBuf, clock: Arc<dyn Clock>) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { db, dir, clock })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub async fn list(&self) -> Result<Vec<Media>> {
        self.db.list_media().await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Media>> {
        self.db.get_media(id).await
    }

    pub async fn import(&self, src: &Path, name: Option<String>) -> Result<Media> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| AppError::Invalid("el archivo no tiene extensión".into()))?;
        let kind = if IMAGE_EXT.contains(&ext.as_str()) {
            MediaKind::Image
        } else if VIDEO_EXT.contains(&ext.as_str()) {
            MediaKind::Video
        } else {
            return Err(AppError::Invalid(format!(
                "formato no admitido (usa {}, {})",
                IMAGE_EXT.join(", "),
                VIDEO_EXT.join(", ")
            )));
        };
        let len = tokio::fs::metadata(src).await?.len();
        if len == 0 || len > MAX_BYTES {
            return Err(AppError::Invalid("el archivo está vacío o supera los 60 MB".into()));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let file = format!("{id}.{ext}");
        let dest = self.dir.join(&file);
        tokio::fs::copy(src, &dest).await?;

        let default_name = src.file_stem().and_then(|s| s.to_str()).unwrap_or("medio");
        let name: String = name.as_deref().unwrap_or(default_name).trim().chars().take(MAX_NAME_CHARS).collect();
        let media = Media {
            id,
            name: if name.is_empty() { "Sin nombre".into() } else { name },
            file,
            kind,
            created_ms: self.clock.now_ms(),
        };
        if let Err(e) = self.db.insert_media(&media).await {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err(e);
        }
        Ok(media)
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        let name: String = name.trim().chars().take(MAX_NAME_CHARS).collect();
        if name.is_empty() {
            return Err(AppError::Invalid("el nombre no puede estar vacío".into()));
        }
        if !self.db.rename_media(id, &name).await? {
            return Err(AppError::Invalid("el medio no existe".into()));
        }
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        if let Some(m) = self.get(id).await? {
            self.db.delete_media(id).await?;
            let _ = tokio::fs::remove_file(self.dir.join(&m.file)).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::clock::AppClock;

    async fn lib() -> (MediaLibrary, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = Db::open_memory().await.expect("db");
        let lib = MediaLibrary::new(db, tmp.path().join("media"), Arc::new(AppClock::new())).expect("lib");
        (lib, tmp)
    }

    fn write(tmp: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let p = tmp.path().join(name);
        std::fs::write(&p, bytes).expect("write");
        p
    }

    #[tokio::test]
    async fn classifies_images_and_videos() {
        let (lib, tmp) = lib().await;
        let img = lib.import(&write(&tmp, "Fuego.GIF", b"GIF89a-datos"), None).await.expect("img");
        let vid = lib.import(&write(&tmp, "clip.mp4", b"\0\0\0 ftypisom"), Some("Mi clip".into())).await.expect("vid");
        assert_eq!((img.kind, img.name.as_str()), (MediaKind::Image, "Fuego"));
        assert_eq!((vid.kind, vid.name.as_str()), (MediaKind::Video, "Mi clip"));
        assert_eq!(img.url_path(), format!("/media/{}", img.file));
        assert_eq!(lib.list().await.expect("list").len(), 2);
    }

    #[tokio::test]
    async fn rejects_dangerous_or_unsupported_files() {
        let (lib, tmp) = lib().await;
        for bad in ["x.svg", "x.html", "x.exe", "x.js", "sin_extension"] {
            assert!(lib.import(&write(&tmp, bad, b"data"), None).await.is_err(), "{bad}");
        }
        assert!(lib.import(&write(&tmp, "vacio.png", b""), None).await.is_err());
        assert!(lib.list().await.expect("list").is_empty());
        assert_eq!(std::fs::read_dir(tmp.path().join("media")).expect("dir").count(), 0);
    }

    #[tokio::test]
    async fn rename_and_delete() {
        let (lib, tmp) = lib().await;
        let m = lib.import(&write(&tmp, "a.png", b"png"), None).await.expect("import");
        lib.rename(&m.id, "  Nuevo ").await.expect("rename");
        assert_eq!(lib.get(&m.id).await.expect("get").expect("some").name, "Nuevo");
        assert!(lib.rename(&m.id, "  ").await.is_err());
        assert!(lib.rename("nope", "x").await.is_err());
        lib.delete(&m.id).await.expect("delete");
        assert!(!lib.dir().join(&m.file).exists());
        lib.delete(&m.id).await.expect("idempotente");
    }
}
