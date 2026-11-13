//! Rutas de archivo que llegan de la interfaz (elegidas en el diálogo de abrir/guardar).
//!
//! Defensa en profundidad: el diálogo siempre da una ruta absoluta, pero un comando no debe aceptar cualquier
//! texto. Toda ruta que entra por un comando pasa por [`user_file`]: absoluta, sin `..` y con una extensión
//! esperada (sin distinguir mayúsculas).

use std::path::{Component, PathBuf};

use crate::error::{AppError, Result};

pub fn user_file(path: &str, extensions: &[&str]) -> Result<PathBuf> {
    let p = PathBuf::from(path);
    if path.trim().is_empty() || !p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(AppError::coded("file.invalidPath", "ruta de archivo no válida"));
    }
    let ext = p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !extensions.contains(&ext.as_str()) {
        let list = format!(".{}", extensions.join(", ."));
        return Err(AppError::coded("file.badExtension", format!("el archivo debe ser {list}")).with("ext", list));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::user_file;
    use crate::error::AppError;

    fn code(r: crate::error::Result<std::path::PathBuf>) -> &'static str {
        match r {
            Err(AppError::Coded { code, .. }) => code,
            other => panic!("se esperaba un error con código: {other:?}"),
        }
    }

    #[test]
    fn accepts_absolute_paths_with_an_expected_extension() {
        let abs = if cfg!(windows) { r"C:\datos\copia.zip" } else { "/datos/copia.zip" };
        assert!(user_file(abs, &["zip"]).is_ok());
        assert!(user_file(&abs.to_uppercase(), &["zip"]).is_ok(), "la extensión no distingue mayúsculas");
        assert!(user_file(&abs.replace(".zip", ".json"), &["zip", "json"]).is_ok());
    }

    #[test]
    fn rejects_relative_paths_traversal_and_other_extensions() {
        let abs = if cfg!(windows) { r"C:\datos\copia.zip" } else { "/datos/copia.zip" };
        assert_eq!(code(user_file("copia.zip", &["zip"])), "file.invalidPath");
        assert_eq!(code(user_file("", &["zip"])), "file.invalidPath");
        assert_eq!(code(user_file(&abs.replace("datos", "datos/../x"), &["zip"])), "file.invalidPath");
        assert_eq!(code(user_file(&abs.replace(".zip", ".exe"), &["zip"])), "file.badExtension");
        assert_eq!(code(user_file(&abs.replace(".zip", ""), &["zip"])), "file.badExtension");
    }
}
