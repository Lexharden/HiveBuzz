use super::*;

fn user(unique: &str) -> User {
    User {
        id: "1".into(),
        unique_id: unique.into(),
        nickname: unique.into(),
        avatar: String::new(),
        is_moderator: false,
        is_subscriber: false,
        is_follower: false,
        team_level: None,
        gifter_level: None,
    }
}

fn member(handle: &str, platform: Option<Platform>) -> Member {
    Member { handle: handle.into(), platform }
}

fn group(id: &str, members: Vec<Member>) -> ViewerGroup {
    ViewerGroup { id: id.into(), name: id.into(), members }
}

fn named(name: &str, members: Vec<Member>) -> ViewerGroup {
    ViewerGroup { id: String::new(), name: name.into(), members }
}

#[test]
fn handles_are_compared_without_case_or_at_sign() {
    assert_eq!(normalize_handle("  @Alexis.Y "), "alexis.y");
    let idx = GroupIndex::new(&[group("vip", vec![member("alexis", None)])]);
    assert!(idx.contains("vip", &user("@ALEXIS"), Platform::Tiktok));
    assert!(!idx.contains("vip", &user("otro"), Platform::Tiktok));
}

#[test]
fn a_member_with_a_platform_only_counts_there() {
    let idx = GroupIndex::new(&[group("vip", vec![member("ana", Some(Platform::Twitch))])]);
    assert!(idx.contains("vip", &user("ana"), Platform::Twitch));
    assert!(!idx.contains("vip", &user("ana"), Platform::Tiktok));
}

#[test]
fn unknown_groups_and_empty_handles_never_match() {
    let idx = GroupIndex::new(&[group("vip", vec![member("ana", None)])]);
    assert!(!idx.contains("borrado", &user("ana"), Platform::Tiktok));
    assert!(!idx.contains("vip", &user("  "), Platform::Tiktok));
    assert!(!GroupIndex::default().contains("vip", &user("ana"), Platform::Tiktok));
}

#[tokio::test]
async fn saving_normalizes_dedupes_and_updates_the_index() {
    let svc = ViewerGroups::new(Db::open_memory().await.expect("db"));
    let members = vec![member("@Ana", None), member("ana", None), member(" ", None), member("ana", Some(Platform::Twitch))];
    let saved = svc.save(named(" VIPs ", members)).await.expect("save");
    assert_eq!(saved.name, "VIPs");
    assert_eq!(saved.members, vec![member("ana", None), member("ana", Some(Platform::Twitch))]);
    assert!(!saved.id.is_empty());
    assert!(svc.index().contains(&saved.id, &user("ANA"), Platform::Tiktok));

    let renamed = svc.save(ViewerGroup { name: "Amigos".into(), ..saved.clone() }).await.expect("rename");
    assert_eq!(renamed.id, saved.id);
    assert_eq!(svc.list().await.len(), 1);
}

#[tokio::test]
async fn names_must_exist_and_be_unique() {
    let svc = ViewerGroups::new(Db::open_memory().await.expect("db"));
    assert!(svc.save(named("  ", vec![])).await.is_err());
    svc.save(named("VIPs", vec![])).await.expect("first");
    assert!(svc.save(named("vips", vec![])).await.is_err());
}

#[tokio::test]
async fn groups_survive_a_restart_and_deleting_empties_the_index() {
    let db = Db::open_memory().await.expect("db");
    let svc = ViewerGroups::new(db.clone());
    let g = svc.save(named("VIPs", vec![member("ana", None)])).await.expect("save");

    let again = ViewerGroups::new(db);
    assert_eq!(again.load().await.expect("load"), 1);
    assert!(again.index().contains(&g.id, &user("ana"), Platform::Tiktok));

    again.delete(&g.id).await.expect("delete");
    assert!(!again.index().contains(&g.id, &user("ana"), Platform::Tiktok));
    assert!(again.list().await.is_empty());
}

#[tokio::test]
async fn limits_are_enforced() {
    let svc = ViewerGroups::new(Db::open_memory().await.expect("db"));
    let many = (0..=MAX_MEMBERS).map(|i| member(&format!("u{i}"), None)).collect();
    assert!(svc.save(named("grande", many)).await.is_err());
    for i in 0..MAX_GROUPS {
        svc.save(named(&format!("g{i}"), vec![])).await.expect("group");
    }
    assert!(svc.save(named("uno más", vec![])).await.is_err());
}
