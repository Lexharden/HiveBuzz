//! HTTPS local para los overlays. TikTok LIVE Studio no acepta URLs `http://` en sus fuentes, así que el
//! servidor también atiende por HTTPS con un certificado propio.
//!
//! Cómo se mantiene seguro:
//! - Se crea una CA propia **restringida por nombre** (X.509 *name constraints*) a `localhost`, `127.0.0.1` y
//!   [`OVERLAY_HOST`] (el dominio que LIVE Studio sí acepta y que apunta a la propia máquina):
//!   aunque el usuario la instale como raíz de confianza, no sirve para suplantar ningún otro sitio.
//! - La clave privada de la CA **no se guarda**: se usa una vez para firmar el certificado del servidor y se
//!   descarta. Lo único que queda en disco es la CA pública, el certificado y la clave del servidor.
//! - Instalar la CA en el sistema es siempre una acción explícita del usuario (botón en Ajustes).

use std::path::{Path, PathBuf};

use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};

use crate::error::{AppError, Result};

/// Nombre de dominio de los overlays. TikTok LIVE Studio rechaza IPs y `localhost`: pide algo con forma de
/// dominio. El registro DNS `A hivebuzz.yafel.dev → 127.0.0.1` lo hace apuntar a la propia máquina.
/// Va en el certificado, en la CA (restricción de nombre) y en la comprobación de `Host`.
pub const OVERLAY_HOST: &str = "hivebuzz.yafel.dev";
/// Línea de la CA que recuerda para qué dominio se generó; si no coincide, se regenera todo.
const HOST_MARK_PREFIX: &str = "# host: ";
const CA_FILE: &str = "hivebuzz-local-ca.pem";
const CERT_FILE: &str = "overlay-cert.pem";
const KEY_FILE: &str = "overlay-key.pem";
/// Prefijo del nombre de la CA; detrás va un sufijo aleatorio para distinguir una generación de otra.
pub const CA_NAME_PREFIX: &str = "HiveBuzz Local CA";
/// Años de validez; pasado este tiempo se genera todo de nuevo (y hay que volver a instalar la CA).
const VALID_YEARS: i32 = 5;
const REGENERATE_AFTER_DAYS: u64 = 365 * 4;

/// Certificados listos para el servidor HTTPS.
#[derive(Clone)]
pub struct LocalTls {
    pub cert_pem: String,
    pub key_pem: String,
    /// Ruta de la CA pública (la que se instala en el sistema).
    pub ca_path: PathBuf,
    /// Nombre (CN) de la CA, para comprobar si está instalada.
    pub ca_name: String,
}

impl LocalTls {
    /// Reutiliza lo que hay en `dir` o lo crea de cero si falta, está incompleto o es muy viejo.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        if let Some(tls) = Self::load(dir) {
            return Ok(tls);
        }
        let fresh = generate()?;
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join(CA_FILE), &fresh.ca_pem)?;
        std::fs::write(dir.join(CERT_FILE), &fresh.cert_pem)?;
        write_private(&dir.join(KEY_FILE), &fresh.key_pem)?;
        Ok(Self { cert_pem: fresh.cert_pem, key_pem: fresh.key_pem, ca_path: dir.join(CA_FILE), ca_name: fresh.ca_name })
    }

    fn load(dir: &Path) -> Option<Self> {
        let ca_path = dir.join(CA_FILE);
        let cert_pem = std::fs::read_to_string(dir.join(CERT_FILE)).ok()?;
        let key_pem = std::fs::read_to_string(dir.join(KEY_FILE)).ok()?;
        let ca_pem = std::fs::read_to_string(&ca_path).ok()?;
        let age = std::fs::metadata(dir.join(CERT_FILE)).ok()?.modified().ok()?.elapsed().ok()?;
        if age.as_secs() > REGENERATE_AFTER_DAYS * 86_400 {
            return None;
        }
        // El nombre va dentro del fichero (comentario) para no tener que analizar el certificado.
        let ca_name = ca_pem.lines().find(|l| l.starts_with("# ") && !l.starts_with(HOST_MARK_PREFIX))?.trim_start_matches("# ").to_string();
        // Generado para otro dominio (p. ej. una versión anterior solo con `localhost`): hay que rehacerlo.
        if !ca_pem.lines().any(|l| l.trim_end() == format!("{HOST_MARK_PREFIX}{OVERLAY_HOST}")) {
            return None;
        }
        Some(Self { cert_pem, key_pem, ca_path, ca_name })
    }
}

struct Fresh {
    ca_pem: String,
    cert_pem: String,
    key_pem: String,
    ca_name: String,
}

fn generate() -> Result<Fresh> {
    let tls = |e: rcgen::Error| AppError::Server(format!("certificado HTTPS: {e}"));
    let suffix: String = (0..8).map(|_| char::from(b"0123456789abcdef"[rand::random_range(0..16)])).collect();
    let ca_name = format!("{CA_NAME_PREFIX} {suffix}");
    let (from, to) = (date_time_ymd(2024, 1, 1), date_time_ymd(2024 + VALID_YEARS + 2, 1, 1));

    let ca_key = KeyPair::generate().map_err(tls)?;
    let mut ca = CertificateParams::default();
    ca.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, ca_name.as_str());
        dn.push(DnType::OrganizationName, "HiveBuzz");
        dn
    };
    ca.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![
            GeneralSubtree::DnsName("localhost".into()),
            GeneralSubtree::DnsName(OVERLAY_HOST.into()),
            GeneralSubtree::IpAddress(CidrSubnet::V4([127, 0, 0, 1], [255, 255, 255, 255])),
        ],
        excluded_subtrees: vec![],
    });
    ca.not_before = from;
    ca.not_after = to;
    let ca_cert = ca.self_signed(&ca_key).map_err(tls)?;

    let leaf_key = KeyPair::generate().map_err(tls)?;
    let mut leaf = CertificateParams::default();
    leaf.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "HiveBuzz overlays");
        dn
    };
    leaf.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into().map_err(tls)?),
        SanType::DnsName(OVERLAY_HOST.try_into().map_err(tls)?),
        SanType::IpAddress(std::net::IpAddr::from([127, 0, 0, 1])),
    ];
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf.not_before = from;
    leaf.not_after = to;
    let issuer = Issuer::new(ca, ca_key);
    let leaf_cert = leaf.signed_by(&leaf_key, &issuer).map_err(tls)?;
    // `ca_key` se descarta aquí: nadie podrá firmar nuevos certificados con la CA instalada.

    Ok(Fresh {
        ca_pem: format!("# {ca_name}\n{HOST_MARK_PREFIX}{OVERLAY_HOST}\n{}", ca_cert.pem()),
        // Cadena completa: el navegador recibe el certificado del servidor y después la CA.
        cert_pem: format!("{}{}", leaf_cert.pem(), ca_cert.pem()),
        key_pem: leaf_key.serialize_pem(),
        ca_name,
    })
}

/// Escribe la clave privada solo legible por el usuario (en Unix; en Windows hereda los permisos del perfil).
fn write_private(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ---- Instalación de la CA en el almacén del sistema ----

/// Resultado de consultar el almacén de confianza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaStatus {
    Installed,
    NotInstalled,
    /// Este sistema no permite comprobarlo ni instalarlo desde la app.
    Unsupported,
}

#[cfg(windows)]
async fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    tokio::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await
        .map_err(|e| AppError::Server(format!("no se pudo ejecutar {program}: {e}")))
}

#[cfg(all(not(windows), target_os = "macos"))]
async fn run(program: &str, args: &[&str]) -> Result<std::process::Output> {
    tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| AppError::Server(format!("no se pudo ejecutar {program}: {e}")))
}

/// ¿Está la CA (esta generación) en el almacén de confianza del usuario?
#[allow(clippy::needless_return)] // cada sistema operativo compila un solo bloque `cfg`, que acaba siendo el final de la función
pub async fn ca_status(ca_name: &str) -> CaStatus {
    #[cfg(windows)]
    {
        return match run("certutil", &["-user", "-store", "Root", ca_name]).await {
            Ok(o) if o.status.success() => CaStatus::Installed,
            _ => CaStatus::NotInstalled,
        };
    }
    #[cfg(target_os = "macos")]
    {
        return match run("security", &["find-certificate", "-c", ca_name]).await {
            Ok(o) if o.status.success() => CaStatus::Installed,
            _ => CaStatus::NotInstalled,
        };
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = ca_name;
        CaStatus::Unsupported
    }
}

/// Instala la CA como raíz de confianza **del usuario actual** (sin permisos de administrador). El sistema
/// pedirá confirmación. Se retiran antes las CA de HiveBuzz de generaciones anteriores.
#[allow(clippy::needless_return)]
pub async fn install_ca(tls: &LocalTls) -> Result<()> {
    #[cfg(windows)]
    {
        // Nada que retirar es un caso normal: se ignora el resultado.
        let _ = run("certutil", &["-user", "-delstore", "Root", CA_NAME_PREFIX]).await;
        let path = tls.ca_path.to_string_lossy().into_owned();
        let out = run("certutil", &["-user", "-addstore", "Root", &path]).await?;
        return if out.status.success() && ca_status(&tls.ca_name).await == CaStatus::Installed {
            Ok(())
        } else {
            Err(AppError::Server("no se instaló el certificado (¿rechazaste el aviso de Windows?)".into()))
        };
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").map_err(|_| AppError::Server("sin carpeta de usuario".into()))?;
        let keychain = format!("{home}/Library/Keychains/login.keychain-db");
        let path = tls.ca_path.to_string_lossy().into_owned();
        let out = run("security", &["add-trusted-cert", "-r", "trustRoot", "-k", &keychain, &path]).await?;
        return if out.status.success() {
            Ok(())
        } else {
            Err(AppError::Server("no se instaló el certificado (¿cancelaste la petición de contraseña?)".into()))
        };
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err(AppError::Server(format!(
            "en este sistema instala a mano el certificado {} como autoridad de confianza",
            tls.ca_path.display()
        )))
    }
}

/// Quita la CA del almacén de confianza del usuario (la de esta generación y las de generaciones anteriores de
/// HiveBuzz que hubieran quedado). Si ya no estaba, no es un error. Los archivos del certificado se conservan:
/// se puede volver a instalar cuando se quiera.
#[allow(clippy::needless_return)]
pub async fn uninstall_ca(tls: &LocalTls) -> Result<()> {
    #[cfg(windows)]
    {
        if ca_status(&tls.ca_name).await == CaStatus::NotInstalled {
            // Puede quedar alguna generación anterior aunque la actual ya no esté.
            let _ = run("certutil", &["-user", "-delstore", "Root", CA_NAME_PREFIX]).await;
            return Ok(());
        }
        let _ = run("certutil", &["-user", "-delstore", "Root", &tls.ca_name]).await?;
        let _ = run("certutil", &["-user", "-delstore", "Root", CA_NAME_PREFIX]).await;
        return if ca_status(&tls.ca_name).await == CaStatus::Installed {
            Err(AppError::Server("no se pudo quitar el certificado (¿rechazaste el aviso de Windows?)".into()))
        } else {
            Ok(())
        };
    }
    #[cfg(target_os = "macos")]
    {
        if ca_status(&tls.ca_name).await == CaStatus::NotInstalled {
            return Ok(());
        }
        let home = std::env::var("HOME").map_err(|_| AppError::Server("sin carpeta de usuario".into()))?;
        let keychain = format!("{home}/Library/Keychains/login.keychain-db");
        let _ = run("security", &["delete-certificate", "-c", &tls.ca_name, "-t", &keychain]).await?;
        return if ca_status(&tls.ca_name).await == CaStatus::Installed {
            Err(AppError::Server("no se pudo quitar el certificado (¿cancelaste la petición de contraseña?)".into()))
        } else {
            Ok(())
        };
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err(AppError::Server(format!(
            "en este sistema quita a mano el certificado {} de tus autoridades de confianza",
            tls.ca_path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_once_and_reuses_the_same_files() {
        let dir = tempfile::tempdir().expect("dir");
        let a = LocalTls::load_or_create(dir.path()).expect("crea");
        let b = LocalTls::load_or_create(dir.path()).expect("reutiliza");
        assert_eq!(a.cert_pem, b.cert_pem);
        assert_eq!(a.key_pem, b.key_pem);
        assert_eq!(a.ca_name, b.ca_name);
        assert!(a.ca_name.starts_with(CA_NAME_PREFIX));
    }

    #[test]
    fn the_ca_private_key_is_never_stored() {
        let dir = tempfile::tempdir().expect("dir");
        LocalTls::load_or_create(dir.path()).expect("crea");
        let ca = std::fs::read_to_string(dir.path().join(CA_FILE)).expect("ca");
        assert!(ca.contains("BEGIN CERTIFICATE") && !ca.contains("PRIVATE KEY"));
        let cert = std::fs::read_to_string(dir.path().join(CERT_FILE)).expect("cert");
        assert!(!cert.contains("PRIVATE KEY"));
        let files: Vec<_> = std::fs::read_dir(dir.path()).expect("ls").flatten().collect();
        assert_eq!(files.len(), 3, "solo CA pública, certificado y clave del servidor");
    }

    #[test]
    fn a_certificate_made_for_another_domain_is_regenerated() {
        // Versiones anteriores solo cubrían `localhost`: su CA no lleva la marca del dominio actual.
        let dir = tempfile::tempdir().expect("dir");
        let a = LocalTls::load_or_create(dir.path()).expect("crea");
        let ca_path = dir.path().join(CA_FILE);
        let old = std::fs::read_to_string(&ca_path).expect("ca").replace(OVERLAY_HOST, "otro.ejemplo.com");
        std::fs::write(&ca_path, old).expect("write");
        let b = LocalTls::load_or_create(dir.path()).expect("regenera");
        assert_ne!(a.ca_name, b.ca_name);
        let ca = std::fs::read_to_string(&ca_path).expect("ca");
        assert!(ca.contains(&format!("{HOST_MARK_PREFIX}{OVERLAY_HOST}")));
    }

    #[test]
    fn an_incomplete_folder_is_regenerated() {
        let dir = tempfile::tempdir().expect("dir");
        let a = LocalTls::load_or_create(dir.path()).expect("crea");
        std::fs::remove_file(dir.path().join(KEY_FILE)).expect("rm");
        let b = LocalTls::load_or_create(dir.path()).expect("regenera");
        assert_ne!(a.ca_name, b.ca_name);
    }
}
