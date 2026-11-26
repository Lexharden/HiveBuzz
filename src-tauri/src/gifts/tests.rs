use std::sync::Arc;

use super::catalog::build;
use super::reference::{parse, Reference, ReferenceGift};
use super::service::{GiftService, ManualGift};
use super::*;
use crate::db::Db;
use crate::events::testing::sample_event;
use crate::events::{EventType, Gift, LiveEvent, Platform};
use crate::testutil::TestClock;

fn r(id: i64, name: &str, diamonds: u64) -> ReferenceGift {
    ReferenceGift { id, name: name.into(), diamonds: Some(diamonds), icon_url: format!("https://cdn/{id}.webp"), image: format!("{id}_{name}.webp") }
}

fn reference() -> Reference {
    Reference::new(
        vec![
            r(5655, "Rose", 1),
            r(5566, "Mishka Bear", 100),
            r(5582, "Mishka Bear", 100),
            r(7000, "Freestyle", 1),
            r(7001, "Freestyle", 1800),
            r(5827, "Ice Cream Cone", 1),
        ],
        None,
    )
}

fn learned(id: Option<i64>, en: Option<&str>, es: Option<&str>, seen: u64) -> LearnedGift {
    LearnedGift {
        key: row_key(id, &[es, en]).expect("clave"),
        gift_id: id,
        name_en: en.map(Into::into),
        name_es: es.map(Into::into),
        seen,
        ..LearnedGift::default()
    }
}

fn gift_event(msg: &str, platform: Platform, id: i64, name: &str, coins: u64, count: u32) -> LiveEvent {
    LiveEvent {
        platform,
        kind: EventType::Gift,
        gift: Some(Gift { id, name: name.into(), coins, count, streakable: true, image: String::new() }),
        chat: None,
        ..sample_event(msg)
    }
}

// ---- Normalización ---------------------------------------------------------------------------------------------

#[test]
fn names_are_compared_without_case_accents_or_signs() {
    assert_eq!(fold("  Corazón "), "corazon");
    assert_eq!(name_key("It’s Corn!"), "itscorn");
    assert!(same_name("CORAZÓN", "corazon"));
    assert_eq!(name_key("🌹"), "🌹", "un nombre sin letras se compara entero");
    assert_eq!(row_key(Some(5655), &[Some("Rose")]).as_deref(), Some("5655"));
    assert_eq!(row_key(None, &[None, Some("Rosa Roja")]).as_deref(), Some("name:rosaroja"));
    assert_eq!(row_key(Some(0), &[None]), None);
    assert_eq!(display_name(Some("Rose"), Some("Rosa"), &[], None), "Rosa / Rose");
    assert_eq!(display_name(Some("Rose"), Some("rose"), &[], None), "rose");
    assert_eq!(display_name(None, None, &[], Some(9)), "#9");
    assert!(looks_spanish("Corazón") && looks_spanish("Rosa") && !looks_spanish("Rose"));
}

// ---- Lista oficial ---------------------------------------------------------------------------------------------

#[test]
fn reference_list_is_read_tolerantly() {
    let list = parse(
        r#"{ "gifts": [
            { "id": 9427, "name": "Pegasus", "diamonds": 42999, "iconUrl": "https://cdn/p.webp", "image": "9427_Pegasus.webp" },
            { "id": "5655", "nombre_en": " Rose ", "diamantes": "1", "icon_url": "http://inseguro", "imagen_local": "gift_images\\5655_Rose.webp" },
            { "id": 9427, "name": "Repetido" },
            { "name": "Sin id" },
            { "id": 0, "name": "Cero" },
            { "id": 12, "name": "Ruta rara", "image": "../../secreto.txt" },
            7
        ] }"#,
    )
    .expect("json");
    assert_eq!(list.len(), 3);
    assert_eq!(list[0], ReferenceGift { id: 9427, name: "Pegasus".into(), diamonds: Some(42999), icon_url: "https://cdn/p.webp".into(), image: "9427_Pegasus.webp".into() });
    assert_eq!(list[1].name, "Rose");
    assert_eq!(list[1].diamonds, Some(1));
    assert_eq!(list[1].icon_url, "", "solo iconos por HTTPS");
    assert_eq!(list[1].image, "5655_Rose.webp", "de una ruta de otra carpeta vale el nombre del archivo");
    assert_eq!(list[2].image, "", "nada que no sea una imagen");
    assert_eq!(parse("[]").expect("json").len(), 0);
    assert_eq!(parse(r#"{"otra":1}"#).expect("json").len(), 0);
    assert!(parse("no es json").is_err());
}

#[test]
fn reference_is_loaded_from_disk_and_missing_images_are_dropped() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::create_dir_all(dir.path().join("images")).expect("dir");
    std::fs::write(dir.path().join("images").join("1_A.webp"), b"x").expect("img");
    std::fs::write(dir.path().join("gifts.json"), r#"[{"id":1,"name":"A","image":"1_A.webp"},{"id":2,"name":"B","image":"2_B.webp"}]"#).expect("json");
    let (reference, problem) = Reference::load(dir.path());
    assert!(problem.is_none());
    assert_eq!(reference.len(), 2);
    assert_eq!(reference.by_id(1).map(|g| g.image.as_str()), Some("1_A.webp"));
    assert_eq!(reference.by_id(2).map(|g| g.image.as_str()), Some(""), "la imagen no existe");
    assert_eq!(reference.by_name("a").map(|g| g.id), Some(1));

    let (empty, problem) = Reference::load(&dir.path().join("no-existe"));
    assert!(empty.is_empty() && problem.is_some());
}

/// La lista que va con la app se lee entera y todas sus imágenes existen.
#[test]
fn bundled_list_is_valid() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("gifts");
    let (reference, problem) = Reference::load(&dir);
    assert!(problem.is_none(), "{problem:?}");
    assert!(reference.len() > 500, "solo {} regalos", reference.len());
    let without_image = reference.gifts.iter().filter(|g| g.image.is_empty()).count();
    assert_eq!(without_image, 0, "regalos sin imagen en la lista de la app");
}

// ---- Catálogo combinado ----------------------------------------------------------------------------------------

#[test]
fn official_list_alone_merges_variants_and_sorts_by_price() {
    let cat = build(&reference(), &[]);
    let names: Vec<_> = cat.gifts.iter().map(|g| (g.display_name.as_str(), g.diamonds)).collect();
    assert_eq!(
        names,
        vec![("Freestyle", Some(1)), ("Ice Cream Cone", Some(1)), ("Rose", Some(1)), ("Mishka Bear", Some(100)), ("Freestyle", Some(1800))]
    );
    let bear = cat.by_id(5582).expect("variante");
    assert_eq!(bear.id, Some(5566), "a igualdad de vistas, el id menor es el principal");
    assert_eq!(bear.other_ids, vec![5582]);
    assert!(bear.reference_only);
    assert_eq!(cat.by_id(7000).map(|g| g.diamonds), Some(Some(1)));
    assert_eq!(cat.by_id(7001).map(|g| g.diamonds), Some(Some(1800)), "mismo nombre, otro precio: otro regalo");
}

#[test]
fn learned_data_is_enriched_with_the_official_list() {
    let cat = build(
        &reference(),
        &[
            // El directo trajo el nombre en español: el oficial pasa a `name_en` y el visto queda como está.
            learned(Some(5655), None, Some("Rosa"), 12),
            // Para el mismo id llegó otro nombre: manda el oficial y el recibido queda como alias.
            learned(Some(5827), Some("Ice Cream"), None, 3),
            // La variante más vista pasa a ser la principal.
            learned(Some(5582), Some("Mishka Bear"), None, 4),
            // Un regalo que no está en la lista oficial.
            LearnedGift { diamonds: Some(77), ..learned(Some(99), Some("Nuevo"), None, 1) },
        ],
    );
    let rose = cat.by_id(5655).expect("rosa");
    assert_eq!((rose.name_en.as_deref(), rose.name_es.as_deref()), (Some("Rose"), Some("Rosa")));
    assert_eq!(rose.display_name, "Rosa / Rose");
    assert_eq!(rose.seen, 12);
    assert!(!rose.reference_only);
    assert_eq!(rose.image, "5655_Rose.webp");
    assert_eq!(rose.diamonds, Some(1), "el precio sale de la lista oficial");

    let cone = cat.by_id(5827).expect("cono");
    assert_eq!(cone.name_en.as_deref(), Some("Ice Cream Cone"));
    assert_eq!(cone.also, vec!["Ice Cream".to_string()]);

    let bear = cat.by_id(5566).expect("oso");
    assert_eq!((bear.id, bear.other_ids.clone(), bear.seen), (Some(5582), vec![5566], 4));

    let new = cat.by_id(99).expect("nuevo");
    assert_eq!((new.diamonds, new.image.as_str()), (Some(77), ""));
}

#[test]
fn search_ignores_accents_and_finds_aliases_and_variant_ids() {
    let cat = build(&reference(), &[learned(Some(5655), None, Some("Corazón Rosa"), 1)]);
    assert_eq!(cat.search("corazon", 10).len(), 1);
    assert_eq!(cat.search("ROSE", 10).len(), 1);
    assert_eq!(cat.search("5582", 10).first().and_then(|g| g.id), Some(5566));
    assert_eq!(cat.search("", 2).len(), 2, "vacío: todos, hasta el límite");
    assert!(cat.search("no existe", 10).is_empty());
    assert_eq!(cat.by_key("5582").and_then(|g| g.id), Some(5566));
    assert_eq!(cat.by_key("rosa").map(|g| g.key.as_str()), None, "«rosa» solo es parte de un nombre");
    assert_eq!(cat.by_key("corazon rosa").and_then(|g| g.id), Some(5655));
}

#[test]
fn matching_accepts_variants_aliases_and_accents() {
    let cat = build(&reference(), &[learned(Some(5655), None, Some("Rosa"), 1)]);
    // Por id: también la variante.
    assert!(cat.matches(Some(5566), None, 5582, "Mishka Bear"));
    assert!(!cat.matches(Some(5566), None, 5655, "Rose"));
    // Por nombre: oficial, en español o con otro texto.
    assert!(cat.matches(None, Some("Rose"), 5655, "Rosa"), "el directo en español y la regla con el nombre oficial");
    assert!(cat.matches(None, Some("rosa"), 5655, "Rose"));
    assert!(cat.matches(None, Some("  mishka-bear "), 5582, "Mishka Bear"));
    assert!(!cat.matches(None, Some("Rose"), 5566, "Mishka Bear"));
    // Id y nombre a la vez: los dos deben cuadrar.
    assert!(cat.matches(Some(7001), Some("Freestyle"), 7001, "Freestyle"));
    assert!(!cat.matches(Some(7001), Some("Freestyle"), 7000, "Freestyle"), "mismo nombre, otro regalo");
    // Sin filtros, cualquiera; un nombre vacío no filtra.
    assert!(cat.matches(None, None, 1, "x"));
    assert!(cat.matches(None, Some("  "), 1, "x"));
    // Evento con un id desconocido pero un nombre que el catálogo relaciona.
    assert!(cat.matches(None, Some("Rosa"), 0, "Rose"));
}

#[test]
fn without_catalog_matching_works_as_before_but_ignores_accents() {
    let cat = build(&Reference::default(), &[]);
    assert!(cat.matches(Some(9), None, 9, "x"));
    assert!(!cat.matches(Some(9), None, 8, "x"));
    assert!(cat.matches(None, Some("GALAXY"), 1, "Galaxy"));
    assert!(cat.matches(None, Some("Corazon"), 1, "Corazón"));
    assert!(!cat.matches(None, Some("Galaxy"), 1, "Lion"));
}

// ---- Servicio --------------------------------------------------------------------------------------------------

async fn service() -> (Arc<GiftService>, Db) {
    let db = Db::open_memory().await.expect("db");
    let svc = GiftService::new(db.clone(), Arc::new(TestClock::default()));
    (svc, db)
}

#[tokio::test]
async fn learns_from_tiktok_gifts_only() {
    let (svc, db) = service().await;
    svc.load(None).await.expect("load");
    svc.observe(&gift_event("1", Platform::Tiktok, 5655, "Rose", 10, 10));
    svc.observe(&gift_event("2", Platform::Tiktok, 5655, "Rosa", 1, 1));
    svc.observe(&gift_event("3", Platform::Twitch, 777, "Bits", 100, 1));
    svc.observe(&gift_event("sim-4", Platform::Tiktok, 888, "Simulado", 1, 1));
    svc.observe(&gift_event("5", Platform::Tiktok, 0, "Sin id", 1, 1));

    // Lo nuevo se ve ya, antes de guardar.
    let rose = svc.get("5655").expect("aprendida");
    assert_eq!((rose.name_en.as_deref(), rose.name_es.as_deref(), rose.diamonds, rose.seen), (Some("Rose"), Some("Rosa"), Some(1), 2));
    assert!(svc.get("777").is_none() && svc.get("888").is_none());

    svc.flush().await.expect("flush");
    let rows = db.list_learned_gifts().await.expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].seen, 2);

    // Al volver a cargar se recupera.
    let again = GiftService::new(db.clone(), Arc::new(TestClock::default()));
    again.load(None).await.expect("load");
    assert_eq!(again.get("5655").map(|g| g.seen), Some(2));
}

#[tokio::test]
async fn learned_names_follow_the_heuristics() {
    let (svc, _db) = service().await;
    svc.load(None).await.expect("load");
    for (i, name) in ["Heart", "Corazón", "Heart Me", "Otro"].iter().enumerate() {
        svc.observe(&gift_event(&i.to_string(), Platform::Tiktok, 42, name, 0, 1));
    }
    let g = svc.get("42").expect("regalo");
    assert_eq!(g.name_en.as_deref(), Some("Heart"));
    assert_eq!(g.name_es.as_deref(), Some("Corazón"));
    assert_eq!(g.also, vec!["Heart Me".to_string(), "Otro".to_string()]);
    assert_eq!(g.diamonds, None, "sin monedas no se inventa un precio");
}

#[tokio::test]
async fn manual_gifts_can_be_created_moved_and_deleted() {
    let (svc, db) = service().await;
    svc.load(None).await.expect("load");

    let err = svc.save_manual(ManualGift::default()).await.expect_err("sin id ni nombre");
    assert!(matches!(err, crate::error::AppError::Coded { code: "gifts.needIdOrName", .. }));
    assert!(svc.save_manual(ManualGift { id: Some(-1), name_en: Some("x".into()), ..ManualGift::default() }).await.is_err());

    // Alta solo con nombre.
    let g = svc.save_manual(ManualGift { name_es: Some(" Rosa Dorada ".into()), diamonds: Some(50), ..ManualGift::default() }).await.expect("alta");
    assert_eq!((g.key.as_str(), g.name_es.as_deref(), g.diamonds, g.manual), ("name:rosadorada", Some("Rosa Dorada"), Some(50), true));

    // Se le pone id: la fila se mueve.
    let g = svc
        .save_manual(ManualGift { previous_key: Some(g.key), id: Some(321), name_es: Some("Rosa Dorada".into()), ..ManualGift::default() })
        .await
        .expect("edición");
    assert_eq!((g.key.as_str(), g.diamonds), ("321", Some(50)), "el precio se conserva si no se cambia");
    let keys: Vec<String> = db.list_learned_gifts().await.expect("rows").into_iter().map(|r| r.key).collect();
    assert_eq!(keys, vec!["321".to_string()]);

    assert!(svc.delete("321").await.expect("borrar"));
    assert!(!svc.delete("321").await.expect("otra vez"));
    assert!(svc.get("321").is_none());
    assert!(db.list_learned_gifts().await.expect("rows").is_empty());
}

#[tokio::test]
async fn bundled_list_loads_through_the_service() {
    let (svc, _db) = service().await;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("gifts");
    svc.load(Some(dir.clone())).await.expect("load");
    assert!(svc.reference_len() > 500);
    assert_eq!(svc.images_dir(), Some(dir.join("images")));
    let rose = svc.search("rose", 50).into_iter().find(|g| g.name_en.as_deref() == Some("Rose")).expect("la rosa");
    assert!(!rose.image.is_empty() && rose.reference_only);
}

#[test]
fn unattached_link_compares_with_an_empty_catalog() {
    let link = super::service::GiftLink::default();
    let cat = link.catalog();
    assert!(cat.is_empty());
    assert!(cat.matches(Some(1), Some("Corazón"), 1, "corazon"), "sin catálogo: id exacto y nombre sin acentos");
    assert!(!cat.matches(Some(1), None, 2, "x"));
}

#[tokio::test]
async fn gifts_without_image_take_the_official_icon_or_the_local_one() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::create_dir_all(dir.path().join("images")).expect("mkdir");
    std::fs::write(dir.path().join("images").join("9_Gato Feliz.webp"), b"x").expect("img");
    std::fs::write(
        dir.path().join("gifts.json"),
        r#"{"gifts":[
            {"id":5655,"name":"Rose","diamonds":1,"iconUrl":"https://cdn/rose.webp","image":""},
            {"id":9,"name":"Gato Feliz","diamonds":5,"iconUrl":"","image":"9_Gato Feliz.webp"}
        ]}"#,
    )
    .expect("json");
    let (svc, _db) = service().await;
    svc.load(Some(dir.path().to_path_buf())).await.expect("load");
    let image = |svc: &GiftService, ev: LiveEvent| {
        let mut ev = ev;
        svc.fill_image(&mut ev);
        ev.gift.map(|g| g.image).unwrap_or_default()
    };

    assert_eq!(image(&svc, gift_event("1", Platform::Tiktok, 5655, "Rose", 1, 1)), "https://cdn/rose.webp");
    assert_eq!(image(&svc, gift_event("2", Platform::Tiktok, 0, "rose", 1, 1)), "https://cdn/rose.webp", "por nombre");
    assert_eq!(image(&svc, gift_event("3", Platform::Tiktok, 9, "Gato Feliz", 5, 1)), "", "sin servidor no hay respaldo local");
    assert_eq!(image(&svc, gift_event("4", Platform::Tiktok, 404, "Desconocido", 1, 1)), "");

    svc.set_local_images("http://127.0.0.1:7777/gifts".into(), "tok".into());
    assert_eq!(image(&svc, gift_event("5", Platform::Tiktok, 9, "Gato Feliz", 5, 1)), "http://127.0.0.1:7777/gifts/9_Gato%20Feliz.webp?token=tok");

    // La imagen que trae el evento manda.
    let mut ev = gift_event("6", Platform::Tiktok, 5655, "Rose", 1, 1);
    if let Some(g) = ev.gift.as_mut() {
        g.image = "https://tiktok/real.webp".into();
    }
    assert_eq!(image(&svc, ev), "https://tiktok/real.webp");
}
