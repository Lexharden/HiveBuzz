use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::actions::clock::AppClock;
use crate::actions::queue::{ActionQueue, QueueConfig};
use crate::actions::store::MemoryJobStore;
use crate::actions::{ActionContext, ActionExecutor, ExecutorRegistry};
use crate::overlay::OverlayHub;
use crate::rules::model::{ActionPlan, ActionSpec, Conditions, PlanMode, Step, Trigger};

struct Noop;

#[async_trait]
impl ActionExecutor for Noop {
    fn kind(&self) -> &'static str {
        "noop"
    }
    async fn execute(&self, _ctx: &ActionContext, _params: &Map<String, Value>) -> Result<()> {
        Ok(())
    }
}

fn rule(id: &str) -> Rule {
    Rule {
        id: id.into(),
        name: format!("regla {id}"),
        enabled: true,
        trigger: Trigger::Follow,
        conditions: Conditions::default(),
        plan: ActionPlan { mode: PlanMode::Sequence, steps: vec![Step { delay_ms: 0, action: ActionSpec::new("noop", json!({})) }] },
        priority: None,
        ttl_ms: 60_000,
        cost_points: None,
    }
}

struct Rig {
    svc: Arc<ProfileService>,
    rules: Arc<RuleEngine>,
    overlays: OverlayConfigService,
    db: Db,
}

async fn rig() -> Rig {
    let db = Db::open_memory().await.unwrap();
    let clock: Arc<dyn Clock> = Arc::new(AppClock::new());
    let mut registry = ExecutorRegistry::new();
    registry.register(Arc::new(Noop));
    let queue = Arc::new(ActionQueue::start(registry.clone(), Arc::new(MemoryJobStore::default()), clock.clone(), QueueConfig::default()));
    let rules = RuleEngine::new(db.clone(), queue, registry, clock.clone());
    let overlays = OverlayConfigService::new(db.clone(), OverlayHub::new(16));
    let svc = ProfileService::new(db.clone(), Arc::clone(&rules), overlays.clone(), clock);
    Rig { svc, rules, overlays, db }
}

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

#[tokio::test]
async fn saving_and_applying_swaps_rules_and_overlay_config() {
    let r = rig().await;
    r.rules.upsert(rule("a")).await.unwrap();
    r.rules.upsert(rule("b")).await.unwrap();
    r.overlays.set("alerts", &patch(json!({ "fontSize": 40 }))).await.unwrap();
    let chat = r.svc.save_current(None, "Charla").await.unwrap();
    assert_eq!((chat.rule_count, chat.name.as_str()), (2, "Charla"));

    // Otro estado: una sola regla y los overlays por defecto.
    r.rules.replace_all(vec![rule("z")]).await.unwrap();
    for def in r.overlays.defs() {
        r.overlays.reset(def.id).await.unwrap();
    }
    let game = r.svc.save_current(None, "Juego").await.unwrap();
    assert_eq!(game.rule_count, 1);

    r.svc.apply(&chat.id).await.unwrap();
    assert_eq!(r.rules.list().iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(r.svc.active().as_deref(), Some(chat.id.as_str()));
    // También quedó en la base de datos, en orden.
    assert_eq!(r.db.list_rules().await.unwrap().iter().map(|x| x.id.clone()).collect::<Vec<_>>(), ["a", "b"]);

    r.svc.apply(&game.id).await.unwrap();
    assert_eq!(r.rules.list().len(), 1);
}

#[tokio::test]
async fn overlay_overrides_roundtrip_through_a_profile() {
    let r = rig().await;
    let before = r.overlays.get("alerts").await.unwrap();
    r.overlays.set("alerts", &patch(json!({ "fontSize": 40 }))).await.unwrap();
    let changed = r.overlays.get("alerts").await.unwrap();
    assert_ne!(changed, before);
    let p = r.svc.save_current(None, "Con cambios").await.unwrap();
    assert_eq!(p.overlay_count, 1, "solo se guarda lo que el usuario cambió");
    r.overlays.reset("alerts").await.unwrap();
    assert_eq!(r.overlays.get("alerts").await.unwrap(), before);
    r.svc.apply(&p.id).await.unwrap();
    assert_eq!(r.overlays.get("alerts").await.unwrap(), changed, "el cambio volvió con el perfil");
}

#[tokio::test]
async fn names_are_validated_and_unique() {
    let r = rig().await;
    assert!(r.svc.save_current(None, "   ").await.is_err());
    let a = r.svc.save_current(None, "Minecraft").await.unwrap();
    assert!(r.svc.save_current(None, "minecraft").await.is_err(), "sin distinguir mayúsculas");
    // Sobrescribir el mismo perfil con su propio nombre sí vale.
    assert!(r.svc.save_current(Some(a.id.clone()), "Minecraft").await.is_ok());
    assert!(r.svc.save_current(Some("no-existe".into()), "X").await.is_err());
    let b = r.svc.save_current(None, "Charla").await.unwrap();
    assert!(r.svc.rename(&b.id, "MINECRAFT").await.is_err());
    r.svc.rename(&b.id, "Charla larga").await.unwrap();
    let names: Vec<String> = r.svc.list().await.unwrap().into_iter().map(|p| p.name).collect();
    assert_eq!(names, ["Charla larga", "Minecraft"]);
}

#[tokio::test]
async fn a_profile_with_an_invalid_rule_changes_nothing() {
    let r = rig().await;
    r.rules.upsert(rule("keep")).await.unwrap();
    let mut bad = rule("bad");
    bad.plan.steps[0].action = ActionSpec::new("tipo-que-no-existe", json!({}));
    let data = ProfileData { rules: vec![bad], overlays: Map::new() };
    r.db.save_profile(&ProfileRow { id: "p".into(), name: "Rota".into(), updated_ms: 0, json: serde_json::to_string(&data).unwrap() })
        .await
        .unwrap();
    assert!(r.svc.apply("p").await.is_err());
    assert_eq!(r.rules.list().iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["keep"]);
    assert_eq!(r.svc.active(), None);
}

#[tokio::test]
async fn duplicated_rule_ids_and_unknown_overlays_are_rejected() {
    let r = rig().await;
    assert!(r.rules.replace_all(vec![rule("a"), rule("a")]).await.is_err());
    let all = patch(json!({ "no-existe": { "x": 1 } }));
    assert!(r.overlays.replace_all_stored(&all).await.is_err());
    let bad_value = patch(json!({ "alerts": { "fontSize": "mucho" } }));
    assert!(r.overlays.replace_all_stored(&bad_value).await.is_err());
}

#[tokio::test]
async fn deleting_the_active_profile_clears_it_and_survives_reload() {
    let r = rig().await;
    let p = r.svc.save_current(None, "Uno").await.unwrap();
    r.svc.apply(&p.id).await.unwrap();
    let again = ProfileService::new(r.db.clone(), Arc::clone(&r.rules), r.overlays.clone(), Arc::new(AppClock::new()));
    again.load().await.unwrap();
    assert_eq!(again.active().as_deref(), Some(p.id.as_str()));
    assert!(r.svc.delete(&p.id).await.unwrap());
    assert_eq!(r.svc.active(), None);
    assert!(!r.svc.delete(&p.id).await.unwrap());
    assert!(r.svc.apply(&p.id).await.is_err());
}
