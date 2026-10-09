use super::*;
use crate::points::logic::{Reason, Stats, Who};

fn who(id: &str, unique: &str, nick: &str) -> Who {
    Who { user_id: id.into(), unique_id: unique.into(), nickname: nick.into(), avatar: String::new(), is_subscriber: false }
}

fn award(id: &str, unique: &str, delta: i64, reason: Reason, stats: Stats, ts: i64) -> Award {
    Award { who: who(id, unique, &unique.to_uppercase()), delta, reason, stats, ts }
}

async fn db() -> Db {
    Db::open_memory().await.expect("db")
}

async fn give(db: &Db, id: &str, unique: &str, points: i64) {
    db.apply_awards(&[award(id, unique, points, Reason::Manual, Stats::default(), 1)]).await.expect("award");
}

#[tokio::test]
async fn awards_create_and_accumulate_a_viewer() {
    let db = db().await;
    let s = |comments, coins| Stats { comments, coins_gifted: coins, ..Stats::default() };
    db.apply_awards(&[award("1", "ana", 5, Reason::Comment, s(1, 0), 100)]).await.expect("a");
    db.apply_awards(&[award("1", "ana", 50, Reason::Gift, s(0, 50), 200), award("1", "ana", 0, Reason::Activity, s(1, 0), 300)]).await.expect("b");
    let v = db.get_viewer("1").await.expect("get").expect("existe");
    assert_eq!((v.points, v.total_earned, v.total_spent), (55, 55, 0));
    assert_eq!((v.comments, v.coins_gifted), (2, 50));
    assert_eq!((v.first_seen_ms, v.last_seen_ms), (100, 300));
    assert_eq!(v.nickname, "ANA");
    // Solo los movimientos con puntos quedan en el historial.
    let h = db.point_history("1", 10).await.expect("hist");
    assert_eq!(h.iter().map(|e| (e.delta, e.reason.as_str())).collect::<Vec<_>>(), [(50, "gift"), (5, "comment")]);
}

#[tokio::test]
async fn a_negative_award_never_drives_the_balance_below_zero() {
    let db = db().await;
    give(&db, "1", "ana", 10).await;
    db.apply_awards(&[award("1", "ana", -30, Reason::Manual, Stats::default(), 2)]).await.expect("neg");
    assert_eq!(db.balance("1").await.expect("bal"), 0);
}

#[tokio::test]
async fn spend_is_all_or_nothing() {
    let db = db().await;
    give(&db, "1", "ana", 100).await;
    assert_eq!(db.spend_points("1", 30, "spend:Sonido", 5).await.expect("spend"), Some(70));
    assert_eq!(db.spend_points("1", 71, "spend:Caro", 6).await.expect("spend"), None);
    assert_eq!(db.balance("1").await.expect("bal"), 70, "un gasto rechazado no toca nada");
    assert_eq!(db.spend_points("nadie", 1, "x", 7).await.expect("spend"), None);
    let v = db.get_viewer("1").await.expect("get").expect("v");
    assert_eq!((v.total_spent, v.total_earned), (30, 100));
    let h = db.point_history("1", 10).await.expect("hist");
    assert_eq!((h[0].delta, h[0].reason.as_str()), (-30, "spend:Sonido"));
    assert_eq!(h.len(), 2, "el rechazo no deja rastro en el historial");
}

#[tokio::test]
async fn concurrent_spends_cannot_overspend() {
    let db = db().await;
    give(&db, "1", "ana", 100).await;
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let d = db.clone();
        tasks.push(tokio::spawn(async move { d.spend_points("1", 30, "spend:x", 9).await.expect("spend") }));
    }
    let mut ok = 0;
    for t in tasks {
        if t.await.expect("join").is_some() {
            ok += 1;
        }
    }
    assert_eq!(ok, 3, "100 puntos alcanzan para exactamente 3 canjes de 30");
    assert_eq!(db.balance("1").await.expect("bal"), 10);
}

#[tokio::test]
async fn manual_adjustments_clamp_and_set_records_the_difference() {
    let db = db().await;
    give(&db, "1", "ana", 50).await;
    assert_eq!(db.adjust_points("1", -80, "manual", 3).await.expect("adj"), 0);
    assert_eq!(db.adjust_points("1", 25, "manual", 4).await.expect("adj"), 25);
    db.set_points("1", 100, 5).await.expect("set");
    assert_eq!(db.balance("1").await.expect("bal"), 100);
    db.set_points("1", 40, 6).await.expect("set");
    assert_eq!(db.balance("1").await.expect("bal"), 40);
    let v = db.get_viewer("1").await.expect("get").expect("v");
    assert_eq!(v.total_earned, 50 + 25 + 75, "50 + 25 + 75 (de 25 a 100)");
    assert_eq!(v.total_spent, 50 + 60, "50 (de 50 a 0) + 60 (de 100 a 40)");
    assert!(db.adjust_points("nadie", 5, "manual", 1).await.is_err());
}

#[tokio::test]
async fn ranking_listing_search_and_pagination() {
    let db = db().await;
    for (i, (u, p)) in [("ana", 300), ("beto", 900), ("cata_99", 50), ("dani", 0)].iter().enumerate() {
        give(&db, &i.to_string(), u, *p).await;
    }
    let top = db.top_viewers_by_points(2).await.expect("top");
    assert_eq!(top.iter().map(|v| v.unique_id.as_str()).collect::<Vec<_>>(), ["beto", "ana"]);
    assert!(db.top_viewers_by_points(10).await.expect("top").iter().all(|v| v.points > 0), "los de 0 no salen en el top");

    let all = db.list_viewers("", SortKey::Points, 10, 0).await.expect("list");
    assert_eq!(all.len(), 4);
    assert_eq!(db.list_viewers("", SortKey::Points, 2, 2).await.expect("page").len(), 2);
    assert_eq!(db.list_viewers("", SortKey::Name, 10, 0).await.expect("list")[0].unique_id, "ana");
    assert_eq!(db.count_viewers("").await.expect("n"), 4);

    let found = db.list_viewers("@ANA", SortKey::Points, 10, 0).await.expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(db.count_viewers("et").await.expect("n"), 1);
}

#[tokio::test]
async fn search_treats_like_wildcards_literally() {
    let db = db().await;
    give(&db, "1", "ana", 1).await;
    give(&db, "2", "beto", 1).await;
    give(&db, "3", "c_t%", 1).await;
    for wildcard in ["%", "_", "a%a", "\\"] {
        let n = db.list_viewers(wildcard, SortKey::Points, 10, 0).await.expect("search").len();
        assert!(n <= 1, "{wildcard:?} no debe comportarse como comodín (devolvió {n})");
    }
    assert_eq!(db.list_viewers("_t%", SortKey::Points, 10, 0).await.expect("s").len(), 1);
}

#[tokio::test]
async fn history_is_pruned_by_age() {
    let db = db().await;
    db.apply_awards(&[award("1", "ana", 5, Reason::Comment, Stats::default(), 100), award("1", "ana", 5, Reason::Comment, Stats::default(), 900)])
        .await
        .expect("a");
    assert_eq!(db.prune_point_history(500).await.expect("prune"), 1);
    assert_eq!(db.point_history("1", 10).await.expect("h").len(), 1);
}

#[tokio::test]
async fn find_by_username_ignores_case_and_at() {
    let db = db().await;
    give(&db, "1", "Ana.M", 10).await;
    assert_eq!(db.find_viewer_by_unique("@ana.m").await.expect("find").expect("v").user_id, "1");
    assert!(db.find_viewer_by_unique("otro").await.expect("find").is_none());
}

// ---- CSV ----

#[tokio::test]
async fn import_creates_provisional_viewers_and_later_merges_them_with_the_real_id() {
    let db = db().await;
    let report = db.import_viewers_csv("unique_id,nickname,points\nana,Ana M,500\nbeto,,20\n", ImportMode::Replace, 10).await.expect("import");
    assert_eq!((report.created, report.updated, report.errors.len()), (2, 0, 0));
    let prov = db.get_viewer("unique:ana").await.expect("get").expect("provisional");
    assert_eq!((prov.points, prov.nickname.as_str()), (500, "Ana M"));

    // Ana habla por primera vez en este programa: el id real absorbe a la fila importada.
    db.apply_awards(&[award("777", "ana", 2, Reason::Comment, Stats { comments: 1, ..Stats::default() }, 20)]).await.expect("award");
    assert!(db.get_viewer("unique:ana").await.expect("get").is_none());
    let real = db.get_viewer("777").await.expect("get").expect("real");
    assert_eq!(real.points, 502, "conserva los puntos importados");
    let h: Vec<_> = db.point_history("777", 10).await.expect("h").into_iter().map(|e| e.reason).collect();
    assert_eq!(h, ["comment", "import"], "el historial pasó al id real");
    assert_eq!(db.count_viewers("").await.expect("n"), 2);
}

#[tokio::test]
async fn import_replace_vs_add_on_existing_viewers() {
    let db = db().await;
    give(&db, "1", "ana", 100).await;
    let r = db.import_viewers_csv("unique_id,points\nana,40\n", ImportMode::Replace, 5).await.expect("import");
    assert_eq!((r.created, r.updated), (0, 1));
    assert_eq!(db.balance("1").await.expect("bal"), 40);
    db.import_viewers_csv("unique_id,points\nANA,10\n", ImportMode::Add, 6).await.expect("import");
    assert_eq!(db.balance("1").await.expect("bal"), 50);
}

#[tokio::test]
async fn import_reports_bad_rows_but_applies_the_good_ones() {
    let db = db().await;
    let r = db.import_viewers_csv("unique_id,points\nana,10\n,5\nbeto,xx\ncata,3\n", ImportMode::Replace, 1).await.expect("import");
    assert_eq!((r.created, r.errors.len()), (2, 2));
    assert_eq!(db.count_viewers("").await.expect("n"), 2);
    assert!(db.import_viewers_csv("sin,columnas\n1,2\n", ImportMode::Replace, 1).await.is_err());
}

#[tokio::test]
async fn export_then_import_into_a_fresh_database_preserves_viewers() {
    let a = db().await;
    a.apply_awards(&[award("1", "ana", 120, Reason::Gift, Stats { coins_gifted: 120, comments: 4, ..Stats::default() }, 1)]).await.expect("a");
    give(&a, "2", "beto", 7).await;
    let csv = a.export_viewers_csv().await.expect("export");

    let b = db().await;
    let r = b.import_viewers_csv(&csv, ImportMode::Replace, 2).await.expect("import");
    assert_eq!((r.created, r.errors.len()), (2, 0));
    let ana = b.find_viewer_by_unique("ana").await.expect("find").expect("ana");
    assert_eq!((ana.points, ana.coins_gifted, ana.comments), (120, 120, 4));
}

#[tokio::test]
async fn deleting_and_clearing() {
    let db = db().await;
    give(&db, "1", "ana", 5).await;
    give(&db, "2", "beto", 5).await;
    assert!(db.delete_viewer("1").await.expect("del"));
    assert!(!db.delete_viewer("1").await.expect("del"));
    assert!(db.point_history("1", 10).await.expect("h").is_empty());
    db.clear_viewers().await.expect("clear");
    assert_eq!(db.count_viewers("").await.expect("n"), 0);
    assert!(db.point_history("2", 10).await.expect("h").is_empty());
}
