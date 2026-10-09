//! Biblioteca local de sonidos: archivos en el directorio de datos + metadatos en SQLite.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::actions::clock::Clock;
use crate::db::Db;
use crate::error::{AppError, Result};

/// Extensiones admitidas (las que sabe decodificar `rodio` con las features activadas).
pub(crate) const EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac"];
const MAX_BYTES: u64 = 25 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sound {
    pub id: String,
    pub name: String,
    /// Nombre del archivo dentro de la carpeta de sonidos (`<uuid>.<ext>`).
    pub file: String,
    /// Volumen propio del sonido, 0–100.
    pub volume: u8,
    pub created_ms: i64,
}

pub struct SoundLibrary {
    db: Db,
    dir: PathBuf,
    clock: Arc<dyn Clock>,
}

impl SoundLibrary {
    pub fn new(db: Db, dir: PathBuf, clock: Arc<dyn Clock>) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { db, dir, clock })
    }

    pub async fn list(&self) -> Result<Vec<Sound>> {
        self.db.list_sounds().await
    }

    pub async fn get(&self, id: &str) -> Result<Option<Sound>> {
        self.db.get_sound(id).await
    }

    /// Ruta absoluta del archivo de un sonido.
    pub fn path_of(&self, sound: &Sound) -> PathBuf {
        self.dir.join(&sound.file)
    }

    /// Copia un archivo de audio a la biblioteca tras comprobar que se puede reproducir.
    pub async fn import(&self, src: &Path, name: Option<String>) -> Result<Sound> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|e| EXTENSIONS.contains(&e.as_str()))
            .ok_or_else(|| {
                AppError::Invalid(format!("formato no admitido (usa {})", EXTENSIONS.join(", ")))
            })?;
        let len = tokio::fs::metadata(src).await?.len();
        if len == 0 || len > MAX_BYTES {
            return Err(AppError::Invalid("el archivo está vacío o supera los 25 MB".into()));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let file = format!("{id}.{ext}");
        let dest = self.dir.join(&file);
        tokio::fs::copy(src, &dest).await?;

        // Si no decodifica, no entra a la biblioteca.
        let probe = dest.clone();
        let decodes = tokio::task::spawn_blocking(move || probe_decodes(&probe))
            .await
            .map_err(|e| AppError::Invalid(e.to_string()))?;
        if let Err(e) = decodes {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err(e);
        }

        let default_name = src.file_stem().and_then(|s| s.to_str()).unwrap_or("sonido");
        let sound = Sound {
            id,
            name: clean_name(name.as_deref().unwrap_or(default_name)),
            file,
            volume: 100,
            created_ms: self.clock.now_ms(),
        };
        if let Err(e) = self.db.insert_sound(&sound).await {
            let _ = tokio::fs::remove_file(&dest).await;
            return Err(e);
        }
        Ok(sound)
    }

    pub async fn update(&self, id: &str, name: Option<String>, volume: Option<u8>) -> Result<Sound> {
        let mut s = self
            .get(id)
            .await?
            .ok_or_else(|| AppError::Invalid("el sonido no existe".into()))?;
        if let Some(n) = name {
            s.name = clean_name(&n);
        }
        if let Some(v) = volume {
            s.volume = v.min(100);
        }
        self.db.update_sound(id, &s.name, s.volume).await?;
        Ok(s)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        if let Some(s) = self.get(id).await? {
            self.db.delete_sound(id).await?;
            // Que el archivo ya no exista no es un error.
            let _ = tokio::fs::remove_file(self.path_of(&s)).await;
        }
        Ok(())
    }
}

fn clean_name(raw: &str) -> String {
    let n: String = raw.trim().chars().take(MAX_NAME_CHARS).collect();
    if n.is_empty() {
        "Sin nombre".to_string()
    } else {
        n
    }
}

fn probe_decodes(path: &Path) -> Result<()> {
    let file = std::fs::File::open(path)?;
    rodio::Decoder::try_from(file)
        .map(|_| ())
        .map_err(|e| AppError::Invalid(format!("no se pudo leer el audio: {e}")))
}

#[cfg(test)]
pub mod testing {
    /// WAV mono de 16 bits con `samples` muestras de silencio.
    pub fn silent_wav(samples: u32) -> Vec<u8> {
        let data_len = samples * 2;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&16000u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.extend(std::iter::repeat_n(0u8, data_len as usize));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::testing::silent_wav;
    use super::*;
    use crate::actions::clock::AppClock;

    async fn lib() -> (SoundLibrary, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = Db::open_memory().await.expect("db");
        let lib = SoundLibrary::new(db, tmp.path().join("sounds"), Arc::new(AppClock::new())).expect("lib");
        (lib, tmp)
    }

    fn write(tmp: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let p = tmp.path().join(name);
        std::fs::write(&p, bytes).expect("write");
        p
    }

    #[tokio::test]
    async fn imports_a_wav_and_lists_it() {
        let (lib, tmp) = lib().await;
        let src = write(&tmp, "Aplausos.WAV", &silent_wav(800));
        let s = lib.import(&src, None).await.expect("import");
        assert_eq!(s.name, "Aplausos");
        assert_eq!(s.volume, 100);
        assert!(s.file.ends_with(".wav"));
        assert!(lib.path_of(&s).exists());
        assert_eq!(lib.list().await.expect("list"), [s]);
    }

    #[tokio::test]
    async fn rejects_unknown_extensions_empty_and_undecodable_files() {
        let (lib, tmp) = lib().await;
        assert!(lib.import(&write(&tmp, "a.txt", b"hola"), None).await.is_err());
        assert!(lib.import(&write(&tmp, "b.wav", b""), None).await.is_err());
        assert!(lib.import(&write(&tmp, "c.mp3", b"esto no es audio de verdad"), None).await.is_err());
        assert!(lib.list().await.expect("list").is_empty());
        let leftovers = std::fs::read_dir(tmp.path().join("sounds")).expect("dir").count();
        assert_eq!(leftovers, 0, "no debe quedar basura tras un import fallido");
    }

    #[tokio::test]
    async fn update_renames_and_clamps_volume() {
        let (lib, tmp) = lib().await;
        let s = lib.import(&write(&tmp, "a.wav", &silent_wav(100)), Some("  Uno  ".into())).await.expect("import");
        assert_eq!(s.name, "Uno");
        let u = lib.update(&s.id, Some("Dos".into()), Some(250)).await.expect("update");
        assert_eq!((u.name.as_str(), u.volume), ("Dos", 100));
        let u = lib.update(&s.id, None, Some(35)).await.expect("update");
        assert_eq!((u.name.as_str(), u.volume), ("Dos", 35));
        assert!(lib.update("nope", None, None).await.is_err());
    }

    #[tokio::test]
    async fn delete_removes_row_and_file() {
        let (lib, tmp) = lib().await;
        let s = lib.import(&write(&tmp, "a.wav", &silent_wav(100)), None).await.expect("import");
        let path = lib.path_of(&s);
        lib.delete(&s.id).await.expect("delete");
        assert!(!path.exists());
        assert!(lib.get(&s.id).await.expect("get").is_none());
        lib.delete(&s.id).await.expect("borrar dos veces no falla");
    }

    #[tokio::test]
    async fn names_are_trimmed_and_bounded() {
        assert_eq!(clean_name("   "), "Sin nombre");
        assert_eq!(clean_name(&"x".repeat(200)).chars().count(), MAX_NAME_CHARS);
    }
}
