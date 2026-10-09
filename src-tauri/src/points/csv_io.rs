//! Importación y exportación CSV de la base de datos de espectadores.
//!
//! Seguridad: los apodos los escriben desconocidos. Al exportar, cualquier celda que empiece por
//! `= + - @` (o tabulador / retorno) se antepone con `'` para que Excel y LibreOffice no la
//! ejecuten como fórmula («CSV injection»).

use std::collections::HashMap;

use super::Viewer;
use crate::error::{AppError, Result};

pub const HEADER: [&str; 9] = [
    "unique_id",
    "nickname",
    "points",
    "total_earned",
    "total_spent",
    "coins_gifted",
    "comments",
    "likes",
    "watch_minutes",
];

/// Máximo de filas en una importación (evita que un archivo enorme congele la app).
pub const MAX_IMPORT_ROWS: usize = 200_000;

/// Neutraliza celdas que una hoja de cálculo interpretaría como fórmula.
pub fn csv_safe(cell: &str) -> String {
    if cell.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{cell}")
    } else {
        cell.to_string()
    }
}

/// Inverso de `csv_safe` para nuestras propias exportaciones.
pub fn csv_unsafe(cell: &str) -> String {
    match cell.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@', '\t', '\r']) => rest.to_string(),
        _ => cell.to_string(),
    }
}

pub fn export(viewers: &[Viewer]) -> Result<String> {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    w.write_record(HEADER).map_err(csv_err)?;
    for v in viewers {
        w.write_record([
            csv_safe(&v.unique_id),
            csv_safe(&v.nickname),
            v.points.to_string(),
            v.total_earned.to_string(),
            v.total_spent.to_string(),
            v.coins_gifted.to_string(),
            v.comments.to_string(),
            v.likes.to_string(),
            v.watch_minutes.to_string(),
        ])
        .map_err(csv_err)?;
    }
    let bytes = w.into_inner().map_err(|e| AppError::Invalid(format!("CSV: {e}")))?;
    String::from_utf8(bytes).map_err(|e| AppError::Invalid(format!("CSV: {e}")))
}

fn csv_err(e: csv::Error) -> AppError {
    AppError::Invalid(format!("CSV: {e}"))
}

/// Una fila válida de una importación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub unique_id: String,
    pub nickname: Option<String>,
    pub points: u64,
    pub total_earned: Option<u64>,
    pub total_spent: Option<u64>,
    pub coins_gifted: Option<u64>,
    pub comments: Option<u64>,
    pub likes: Option<u64>,
    pub watch_minutes: Option<u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub rows: Vec<ImportRow>,
    /// Líneas descartadas, con el motivo (en español, listas para mostrar).
    pub errors: Vec<String>,
}

/// Lee un CSV. Acepta cualquier orden de columnas y columnas de menos; necesita `unique_id` (o
/// `username` / `user`) y `points`. Las filas malas se saltan y se reportan, el resto se importa.
pub fn parse(text: &str) -> Result<Parsed> {
    let text = text.trim_start_matches('\u{feff}'); // BOM de Excel
    let delimiter = if text.lines().next().is_some_and(|l| l.matches(';').count() > l.matches(',').count()) { b';' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).trim(csv::Trim::All).from_reader(text.as_bytes());

    let headers: Vec<String> = rdr.headers().map_err(csv_err)?.iter().map(|h| h.trim().to_lowercase().replace([' ', '-'], "_")).collect();
    let col = |names: &[&str]| headers.iter().position(|h| names.contains(&h.as_str()));
    let id_col = col(&["unique_id", "uniqueid", "username", "user", "usuario"]).ok_or_else(|| AppError::Invalid("falta la columna «unique_id» (o «username»)".into()))?;
    let points_col = col(&["points", "puntos"]).ok_or_else(|| AppError::Invalid("falta la columna «points»".into()))?;
    let cols: HashMap<&str, Option<usize>> = [
        ("nickname", col(&["nickname", "name", "nombre", "apodo"])),
        ("total_earned", col(&["total_earned"])),
        ("total_spent", col(&["total_spent"])),
        ("coins_gifted", col(&["coins_gifted", "coins"])),
        ("comments", col(&["comments"])),
        ("likes", col(&["likes"])),
        ("watch_minutes", col(&["watch_minutes", "watchtime"])),
    ]
    .into_iter()
    .collect();

    let mut out = Parsed::default();
    for (i, rec) in rdr.records().enumerate() {
        let line = i + 2; // la 1 es la cabecera
        if i >= MAX_IMPORT_ROWS {
            out.errors.push(format!("se alcanzó el máximo de {MAX_IMPORT_ROWS} filas; el resto se ignoró"));
            break;
        }
        let rec = match rec {
            Ok(r) => r,
            Err(e) => {
                out.errors.push(format!("línea {line}: {e}"));
                continue;
            }
        };
        let get = |idx: Option<usize>| idx.and_then(|c| rec.get(c)).map(str::trim).filter(|s| !s.is_empty());
        let unique_id = csv_unsafe(get(Some(id_col)).unwrap_or("")).trim().trim_start_matches('@').to_lowercase();
        if unique_id.is_empty() || unique_id.len() > 64 {
            out.errors.push(format!("línea {line}: usuario vacío o demasiado largo"));
            continue;
        }
        let Some(points) = get(Some(points_col)).and_then(parse_count) else {
            out.errors.push(format!("línea {line}: puntos no válidos para «{unique_id}»"));
            continue;
        };
        let opt = |name: &str| cols.get(name).copied().flatten().and_then(|c| get(Some(c))).and_then(parse_count);
        out.rows.push(ImportRow {
            unique_id,
            nickname: get(cols.get("nickname").copied().flatten()).map(|n| csv_unsafe(n).chars().take(80).collect()),
            points,
            total_earned: opt("total_earned"),
            total_spent: opt("total_spent"),
            coins_gifted: opt("coins_gifted"),
            comments: opt("comments"),
            likes: opt("likes"),
            watch_minutes: opt("watch_minutes"),
        });
    }
    Ok(out)
}

/// Entero no negativo. Excel suele exportar «1500.0»; los separadores de miles («1.500», «1,500»)
/// se rechazan porque son ambiguos entre idiomas.
fn parse_count(s: &str) -> Option<u64> {
    let t = s.trim();
    // Excel suele exportar enteros como «1500.0».
    let t = t.strip_suffix(".0").unwrap_or(t);
    t.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(unique: &str, nick: &str, points: u64) -> Viewer {
        Viewer {
            user_id: "1".into(),
            unique_id: unique.into(),
            nickname: nick.into(),
            avatar: String::new(),
            points,
            total_earned: points + 10,
            total_spent: 10,
            coins_gifted: 5,
            comments: 3,
            likes: 7,
            shares: 0,
            watch_minutes: 60,
            first_seen_ms: 0,
            last_seen_ms: 0,
        }
    }

    #[test]
    fn dangerous_cells_are_neutralised_and_restored() {
        for c in ["=cmd|' /C calc'!A0", "+1+1", "-2", "@SUM(A1)", "\tx", "\rx"] {
            let safe = csv_safe(c);
            assert!(safe.starts_with('\''), "{c:?}");
            assert_eq!(csv_unsafe(&safe), c);
        }
        assert_eq!(csv_safe("Ana"), "Ana");
        assert_eq!(csv_unsafe("'normal"), "'normal", "una comilla legítima se conserva");
    }

    #[test]
    fn export_writes_header_rows_and_protects_formulas() {
        let csv = export(&[viewer("ana", "Ana", 100), viewer("evil", "=HYPERLINK(\"http://x\")", 5)]).expect("export");
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], HEADER.join(","));
        assert!(lines[1].starts_with("ana,Ana,100,110,10,5,3,7,60"), "{}", lines[1]);
        assert!(lines[2].contains("'=HYPERLINK"), "{}", lines[2]);
        assert!(!lines[2].contains(",=HYPERLINK"));
    }

    #[test]
    fn export_then_parse_roundtrips() {
        let original = vec![viewer("ana", "Ana, la \"Grande\"", 100), viewer("beto", "Beto", 7)];
        let parsed = parse(&export(&original).expect("export")).expect("parse");
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].unique_id, "ana");
        assert_eq!(parsed.rows[0].nickname.as_deref(), Some("Ana, la \"Grande\""));
        assert_eq!((parsed.rows[0].points, parsed.rows[0].total_earned, parsed.rows[0].watch_minutes), (100, Some(110), Some(60)));
    }

    #[test]
    fn formula_nicknames_survive_a_roundtrip() {
        let parsed = parse(&export(&[viewer("x", "=2+2", 1)]).expect("export")).expect("parse");
        assert_eq!(parsed.rows[0].nickname.as_deref(), Some("=2+2"));
    }

    #[test]
    fn accepts_other_tools_formats() {
        // Orden distinto, punto y coma, BOM, mayúsculas, @ y «1500.0».
        let text = "\u{feff}Usuario;Puntos;Nombre\n@Ana;1500.0;Ana M\nBETO;20;\n";
        let p = parse(text).expect("parse");
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!((p.rows[0].unique_id.as_str(), p.rows[0].points), ("ana", 1500));
        assert_eq!(p.rows[0].nickname.as_deref(), Some("Ana M"));
        assert_eq!((p.rows[1].unique_id.as_str(), p.rows[1].nickname.clone()), ("beto", None));
    }

    #[test]
    fn bad_rows_are_skipped_and_reported_without_aborting() {
        let text = "unique_id,points\nana,10\n,5\nbeto,-3\ncata,mucho\ndani,7\n";
        let p = parse(text).expect("parse");
        assert_eq!(p.rows.iter().map(|r| r.unique_id.as_str()).collect::<Vec<_>>(), ["ana", "dani"]);
        assert_eq!(p.errors.len(), 3);
        assert!(p.errors[0].starts_with("línea 3"));
        assert!(p.errors[1].contains("beto"));
    }

    #[test]
    fn missing_required_columns_are_a_clear_error() {
        assert!(parse("nombre,otra\nx,1\n").expect_err("sin id").to_string().contains("unique_id"));
        assert!(parse("unique_id,otra\nx,1\n").expect_err("sin puntos").to_string().contains("points"));
        assert!(parse("").is_err());
    }

    #[test]
    fn short_rows_and_extra_columns_are_tolerated() {
        let p = parse("unique_id,points,nickname,extra\nana,5\nbeto,6,B,zzz,otro\n").expect("parse");
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0].nickname, None);
    }

    #[test]
    fn absurd_usernames_are_rejected() {
        let long = "x".repeat(65);
        let p = parse(&format!("unique_id,points\n{long},1\n")).expect("parse");
        assert!(p.rows.is_empty());
        assert_eq!(p.errors.len(), 1);
    }
}
