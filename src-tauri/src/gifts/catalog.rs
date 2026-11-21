//! Catálogo combinado (lista oficial + lo aprendido) e índice para comparar regalos.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use super::reference::{Reference, ReferenceGift};
use super::{display_name, fold, name_key, same_name, GiftView, LearnedGift};

/// Foto inmutable del catálogo. Se reconstruye entera cuando cambia algo (son unos cientos de regalos).
#[derive(Debug, Default)]
pub struct Catalog {
    /// Ordenados por diamantes y nombre.
    pub gifts: Vec<GiftView>,
    /// Cualquier id (también de variantes) → posición en `gifts`.
    by_id: HashMap<i64, usize>,
    /// Clave de cualquier nombre o alias → posiciones (hay regalos distintos con el mismo nombre).
    by_name: HashMap<String, Vec<usize>>,
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.gifts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.gifts.is_empty()
    }

    /// Regalo por cualquiera de sus ids.
    pub fn by_id(&self, id: i64) -> Option<&GiftView> {
        self.by_id.get(&id).map(|&i| &self.gifts[i])
    }

    /// Regalo por su clave de fila, su id en texto o (si no) el primero con ese nombre.
    pub fn by_key(&self, key: &str) -> Option<&GiftView> {
        let key = key.trim();
        if let Ok(id) = key.parse::<i64>() {
            return self.by_id(id);
        }
        self.gifts.iter().find(|g| g.key == key).or_else(|| self.by_name(key))
    }

    /// El primer regalo (el más barato) con ese nombre o alias.
    pub fn by_name(&self, name: &str) -> Option<&GiftView> {
        self.by_name.get(&name_key(name)).and_then(|v| v.first()).map(|&i| &self.gifts[i])
    }

    /// Para el buscador de la interfaz: hasta `limit` regalos en el orden del catálogo.
    pub fn search(&self, query: &str, limit: usize) -> Vec<GiftView> {
        let q = fold(query);
        self.gifts.iter().filter(|g| g.matches_query(&q)).take(limit).cloned().collect()
    }

    /// El regalo de un evento (por id y, si no se conoce, por nombre).
    pub fn for_event(&self, gift_id: i64, gift_name: &str) -> Option<&GiftView> {
        self.by_id(gift_id).or_else(|| self.by_name(gift_name))
    }

    /// ¿El regalo de un evento es el que pide una regla, meta o timer?
    ///
    /// - `want_id` acepta también las variantes del mismo regalo (otro id, mismo nombre y precio).
    /// - `want_name` se compara sin mayúsculas, acentos ni signos, y acepta cualquier nombre o alias del regalo
    ///   («Corazón» y «Heart» son el mismo si el catálogo lo sabe).
    ///
    /// Sin catálogo se comporta como antes (id exacto y nombre sin distinguir mayúsculas), solo que sin acentos.
    pub fn matches(&self, want_id: Option<i64>, want_name: Option<&str>, gift_id: i64, gift_name: &str) -> bool {
        let id_ok = want_id.is_none_or(|want| want == gift_id || self.same_gift(want, gift_id));
        let name_ok = want_name.map(str::trim).filter(|n| !n.is_empty()).is_none_or(|want| {
            let want = name_key(want);
            want == name_key(gift_name) || self.has_name(gift_id, &want) || self.share_name(&want, gift_name)
        });
        id_ok && name_ok
    }

    fn same_gift(&self, a: i64, b: i64) -> bool {
        matches!((self.by_id.get(&a), self.by_id.get(&b)), (Some(x), Some(y)) if x == y)
    }

    /// ¿El regalo con ese id tiene ese nombre (clave) entre sus nombres o alias?
    fn has_name(&self, gift_id: i64, key: &str) -> bool {
        self.by_id(gift_id).is_some_and(|g| g.names().any(|n| name_key(n) == key))
    }

    /// ¿Algún regalo tiene a la vez ese nombre pedido y el nombre recibido? (evento sin id conocido)
    fn share_name(&self, want_key: &str, gift_name: &str) -> bool {
        match (self.by_name.get(want_key), self.by_name.get(&name_key(gift_name))) {
            (Some(a), Some(b)) => a.iter().any(|i| b.contains(i)),
            _ => false,
        }
    }
}

/// Combina la lista oficial con lo aprendido y las altas manuales.
pub fn build(reference: &Reference, learned: &[LearnedGift]) -> Catalog {
    let mut views = Vec::with_capacity(learned.len() + reference.len());
    let mut known = HashSet::new();
    for row in learned {
        views.push(enrich(row, reference));
        if let Some(id) = row.gift_id {
            known.insert(id);
        }
    }
    for r in &reference.gifts {
        if known.insert(r.id) {
            views.push(from_reference(r));
        }
    }
    let mut gifts = merge_variants(views);
    for g in &mut gifts {
        g.display_name = display_name(g.name_en.as_deref(), g.name_es.as_deref(), &g.also, g.id);
    }
    gifts.sort_by(|a, b| {
        cmp_diamonds(a.diamonds, b.diamonds)
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
            .then_with(|| a.id.unwrap_or(i64::MAX).cmp(&b.id.unwrap_or(i64::MAX)))
    });

    let mut by_id = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, g) in gifts.iter().enumerate() {
        for id in g.ids() {
            by_id.entry(id).or_insert(i);
        }
        for n in g.names() {
            let key = name_key(n);
            if key.is_empty() {
                continue;
            }
            let slot = by_name.entry(key).or_default();
            if !slot.contains(&i) {
                slot.push(i);
            }
        }
    }
    Catalog { gifts, by_id, by_name }
}

/// Sin precio, al final.
fn cmp_diamonds(a: Option<u64>, b: Option<u64>) -> Ordering {
    a.unwrap_or(u64::MAX).cmp(&b.unwrap_or(u64::MAX))
}

fn view_of(row: &LearnedGift) -> GiftView {
    GiftView {
        key: row.key.clone(),
        id: row.gift_id,
        other_ids: Vec::new(),
        name_en: row.name_en.clone(),
        name_es: row.name_es.clone(),
        also: row.also.clone(),
        display_name: String::new(),
        diamonds: row.diamonds.filter(|d| *d > 0),
        seen: row.seen,
        last_seen: row.last_seen,
        image: String::new(),
        icon_url: String::new(),
        reference_only: false,
        manual: row.manual,
    }
}

/// Lo aprendido, completado con la lista oficial: precio si falta, imagen, y el nombre oficial (el del directo
/// queda como alias: TikTok renombra regalos y a veces manda otro nombre para el mismo id).
fn enrich(row: &LearnedGift, reference: &Reference) -> GiftView {
    let mut view = view_of(row);
    let primary = row.name_es.as_deref().or(row.name_en.as_deref()).or(row.also.first().map(String::as_str));
    let entry = match row.gift_id {
        Some(id) => reference.by_id(id),
        None => primary.and_then(|n| reference.by_name(n)),
    };
    let Some(entry) = entry else {
        return view;
    };
    if view.diamonds.is_none() {
        view.diamonds = entry.diamonds.filter(|d| *d > 0);
    }
    if !entry.name.is_empty() && !view.name_en.as_deref().is_some_and(|n| same_name(n, &entry.name)) {
        if let Some(seen_name) = view.name_en.take() {
            let dup = view.name_es.as_deref().is_some_and(|es| same_name(es, &seen_name)) || view.also.iter().any(|a| same_name(a, &seen_name));
            if !dup {
                view.also.push(seen_name);
            }
        }
        view.also.retain(|a| !same_name(a, &entry.name));
        view.name_en = Some(entry.name.clone());
    }
    view.image.clone_from(&entry.image);
    view.icon_url.clone_from(&entry.icon_url);
    view
}

fn from_reference(r: &ReferenceGift) -> GiftView {
    GiftView {
        key: r.id.to_string(),
        id: Some(r.id),
        other_ids: Vec::new(),
        name_en: (!r.name.is_empty()).then(|| r.name.clone()),
        name_es: None,
        also: Vec::new(),
        display_name: String::new(),
        diamonds: r.diamonds.filter(|d| *d > 0),
        seen: 0,
        last_seen: None,
        image: r.image.clone(),
        icon_url: r.icon_url.clone(),
        reference_only: true,
        manual: false,
    }
}

/// TikTok publica variantes del mismo regalo con otro id (por región o evento): mismo nombre y mismo precio. Se
/// juntan en una fila (el id más visto como principal, los demás en `other_ids`, vistas sumadas). Con distinto
/// precio son regalos distintos que comparten nombre (Freestyle de 1 y de 1800) y quedan separados.
fn merge_variants(views: Vec<GiftView>) -> Vec<GiftView> {
    let mut groups: Vec<Vec<GiftView>> = Vec::new();
    let mut slot_of: HashMap<String, usize> = HashMap::new();
    for v in views {
        let name = v.name_en.as_deref().or(v.name_es.as_deref()).or(v.also.first().map(String::as_str)).map(name_key).unwrap_or_default();
        let group = match v.diamonds {
            Some(d) if !name.is_empty() => format!("{name}|{d}"),
            _ => format!("solo|{}", v.key),
        };
        match slot_of.get(&group) {
            Some(&i) => groups[i].push(v),
            None => {
                slot_of.insert(group, groups.len());
                groups.push(vec![v]);
            }
        }
    }

    groups
        .into_iter()
        .filter_map(|mut group| {
            group.sort_by(|a, b| b.seen.cmp(&a.seen).then_with(|| a.id.unwrap_or(i64::MAX).cmp(&b.id.unwrap_or(i64::MAX))));
            let mut it = group.into_iter();
            let mut main = it.next()?;
            for variant in it {
                for id in variant.ids() {
                    if Some(id) != main.id && !main.other_ids.contains(&id) {
                        main.other_ids.push(id);
                    }
                }
                for name in variant.names() {
                    let known = main.names().any(|n| same_name(n, name));
                    if !known {
                        main.also.push(name.to_string());
                    }
                }
                main.seen += variant.seen;
                main.reference_only &= variant.reference_only;
                main.manual |= variant.manual;
                if variant.last_seen > main.last_seen {
                    main.last_seen = variant.last_seen;
                }
                if main.image.is_empty() {
                    main.image = variant.image;
                }
                if main.icon_url.is_empty() {
                    main.icon_url = variant.icon_url;
                }
            }
            main.other_ids.sort_unstable();
            Some(main)
        })
        .collect()
}
