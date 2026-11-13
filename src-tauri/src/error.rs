use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

/// Error común de la app.
///
/// Hacia la UI viaja como objeto (`{ kind, message, detail?, code?, params? }`, ver [`Wire`]) para que la interfaz
/// pueda traducirlo: un error [`AppError::Coded`] se traduce entero por su `code`; los demás, al menos el tipo
/// (`kind`). `message` es siempre el texto completo en español, el mismo que va al registro.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("sidecar: {0}")]
    Sidecar(String),
    #[error("base de datos: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migraciones: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("llavero del sistema: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("servidor local: {0}")]
    Server(String),
    #[error("{0}")]
    Invalid(String),
    /// Error con código: la UI lo traduce con la clave `errors.code.<code>` y sus `params`.
    #[error("{message}")]
    Coded { code: &'static str, params: Vec<(&'static str, String)>, message: String },
}

impl AppError {
    /// Error con código traducible; `message` es el texto en español (registro y respaldo si falta la traducción).
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self::Coded { code, params: Vec::new(), message: message.into() }
    }

    /// Añade un parámetro de la traducción (`{{name}}` en el texto de i18n).
    #[must_use]
    pub fn with(mut self, name: &'static str, value: impl Into<String>) -> Self {
        if let Self::Coded { params, .. } = &mut self {
            params.push((name, value.into()));
        }
        self
    }

    /// Tipo del error para la UI.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sidecar(_) => "sidecar",
            Self::Db(_) => "db",
            Self::Migrate(_) => "migrate",
            Self::Keyring(_) => "keyring",
            Self::Json(_) => "json",
            Self::Io(_) => "io",
            Self::Server(_) => "server",
            Self::Invalid(_) | Self::Coded { .. } => "invalid",
        }
    }

    /// El texto sin el prefijo del tipo (para que la UI ponga el prefijo traducido).
    fn detail(&self) -> Option<String> {
        match self {
            Self::Sidecar(s) | Self::Server(s) => Some(s.clone()),
            Self::Db(e) => Some(e.to_string()),
            Self::Migrate(e) => Some(e.to_string()),
            Self::Keyring(e) => Some(e.to_string()),
            Self::Json(e) => Some(e.to_string()),
            Self::Io(e) => Some(e.to_string()),
            Self::Invalid(_) | Self::Coded { .. } => None,
        }
    }
}

/// Forma en la que el error llega a la UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    kind: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    params: Map<String, Value>,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let (code, params) = match self {
            Self::Coded { code, params, .. } => {
                (Some(*code), params.iter().map(|(k, v)| ((*k).to_string(), Value::String(v.clone()))).collect())
            }
            _ => (None, Map::new()),
        };
        Wire { kind: self.kind(), message: self.to_string(), detail: self.detail(), code, params }.serialize(s)
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn plain_errors_carry_kind_message_and_detail() {
        let e = AppError::Io(std::io::Error::other("disco lleno"));
        assert_eq!(serde_json::to_value(&e).expect("json"), json!({ "kind": "io", "message": "E/S: disco lleno", "detail": "disco lleno" }));
        let e = AppError::Invalid("el sonido no existe".into());
        assert_eq!(serde_json::to_value(&e).expect("json"), json!({ "kind": "invalid", "message": "el sonido no existe" }));
    }

    #[test]
    fn coded_errors_carry_code_and_params_and_keep_the_spanish_text() {
        let e = AppError::coded("file.badExtension", "el archivo debe terminar en .zip").with("ext", ".zip");
        assert_eq!(e.to_string(), "el archivo debe terminar en .zip");
        assert_eq!(
            serde_json::to_value(&e).expect("json"),
            json!({ "kind": "invalid", "message": "el archivo debe terminar en .zip", "code": "file.badExtension", "params": { "ext": ".zip" } })
        );
        // `with` sobre un error sin código no hace nada.
        assert!(matches!(AppError::Invalid("x".into()).with("a", "b"), AppError::Invalid(_)));
    }
}
