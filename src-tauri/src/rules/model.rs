//! Modelo declarativo de reglas: trigger → condiciones → acciones. Se guarda como JSON.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub trigger: Trigger,
    #[serde(default)]
    pub conditions: Conditions,
    pub plan: ActionPlan,
    /// Prioridad en la cola; si falta se calcula (los regalos grandes se adelantan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    /// Caducidad de las acciones en cola, en ms.
    #[serde(default = "default_ttl_ms")]
    pub ttl_ms: u64,
    /// Coste en puntos del espectador que la dispara: así una regla es una **recompensa** canjeable.
    /// Se comprueba el saldo antes de consumir cooldowns y se descuenta de forma atómica.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_points: Option<u64>,
}

fn default_true() -> bool {
    true
}

fn default_ttl_ms() -> u64 {
    60_000
}

/// Qué dispara la regla.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Trigger {
    /// Regalo concreto (por id y/o nombre) y/o con un mínimo de monedas (valor total).
    #[serde(rename_all = "camelCase")]
    Gift {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gift_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min_coins: Option<u64>,
    },
    Follow,
    Share,
    Subscribe,
    Join,
    /// Emote enviado por un suscriptor.
    SubEmote,
    /// Cada `every` likes acumulados.
    Like { every: u64 },
    /// Comando de chat, p. ej. `!sonido` (con o sin `!`).
    Command { command: String },
    /// Palabras clave en el chat.
    #[serde(rename_all = "camelCase")]
    Keyword {
        keywords: Vec<String>,
        #[serde(default)]
        whole_word: bool,
    },
    /// Una meta (Fase 3) alcanzó su objetivo.
    #[serde(rename_all = "camelCase")]
    GoalReached { goal_id: String },
    /// Un timer (Fase 3) terminó.
    #[serde(rename_all = "camelCase")]
    TimerEnded { timer_id: String },
    /// Llamada a la API local (`POST /api/trigger` con este `name`). No se dispara con eventos del LIVE.
    Api { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Moderator,
    Subscriber,
    Follower,
}

/// Condiciones adicionales; todas deben cumplirse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conditions {
    #[serde(default)]
    pub global_cooldown_ms: u64,
    #[serde(default)]
    pub user_cooldown_ms: u64,
    /// Basta con tener uno de estos roles. Vacío = cualquiera.
    #[serde(default)]
    pub roles_any: Vec<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_team_level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_gifter_level: Option<u32>,
    /// Probabilidad de ejecución, 0–100.
    #[serde(default = "default_probability")]
    pub probability: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
}

fn default_probability() -> f64 {
    100.0
}

impl Default for Conditions {
    fn default() -> Self {
        Self {
            global_cooldown_ms: 0,
            user_cooldown_ms: 0,
            roles_any: Vec::new(),
            min_team_level: None,
            min_gifter_level: None,
            probability: 100.0,
            schedule: None,
        }
    }
}

/// Ventana horaria local. `days`: 0 = lunes … 6 = domingo (vacío = todos).
/// Si `from > to` la ventana cruza la medianoche.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(default)]
    pub days: Vec<u8>,
    /// "HH:MM"
    pub from: String,
    /// "HH:MM"
    pub to: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanMode {
    /// Una acción tras otra, respetando los retardos.
    #[default]
    Sequence,
    /// Todas a la vez (cada una con su propio retardo).
    Parallel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPlan {
    #[serde(default)]
    pub mode: PlanMode,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    #[serde(default)]
    pub delay_ms: u64,
    pub action: ActionSpec,
}

/// Acción abierta: `{"type": "<tipo>", ...parámetros}`. Cada `ActionExecutor` interpreta los suyos,
/// así añadir un ejecutor no toca el núcleo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub params: Map<String, Value>,
}

impl ActionSpec {
    pub fn new(kind: &str, params: Value) -> Self {
        let params = match params {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        Self {
            kind: kind.to_string(),
            params,
        }
    }

    pub fn params_value(&self) -> Value {
        Value::Object(self.params.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_a_full_rule_from_json() {
        let rule: Rule = serde_json::from_value(json!({
            "id": "r1", "name": "Rosa",
            "trigger": {"type": "gift", "giftName": "Rose", "minCoins": 1},
            "conditions": {"userCooldownMs": 5000, "rolesAny": ["subscriber"], "probability": 50},
            "plan": {"mode": "parallel", "steps": [
                {"action": {"type": "playSound", "soundId": "s1", "volume": 80}},
                {"delayMs": 500, "action": {"type": "tts", "text": "Gracias {nickname}"}}
            ]}
        }))
        .expect("parsea");
        assert!(rule.enabled);
        assert_eq!(rule.ttl_ms, 60_000);
        assert_eq!(rule.conditions.user_cooldown_ms, 5000);
        assert_eq!(rule.conditions.roles_any, [Role::Subscriber]);
        assert_eq!(rule.plan.mode, PlanMode::Parallel);
        assert_eq!(rule.plan.steps[0].action.kind, "playSound");
        assert_eq!(rule.plan.steps[0].action.params["volume"], 80);
        assert_eq!(rule.plan.steps[1].delay_ms, 500);
    }

    #[test]
    fn minimal_rule_gets_sane_defaults() {
        let rule: Rule = serde_json::from_value(json!({
            "id": "r", "name": "n", "trigger": {"type": "follow"}, "plan": {"steps": []}
        }))
        .expect("parsea");
        assert_eq!(rule.conditions, Conditions::default());
        assert_eq!(rule.conditions.probability, 100.0);
        assert_eq!(rule.plan.mode, PlanMode::Sequence);
    }

    #[test]
    fn rule_roundtrips_through_json() {
        let rule = Rule {
            id: "r".into(),
            name: "n".into(),
            enabled: false,
            trigger: Trigger::Like { every: 100 },
            conditions: Conditions::default(),
            plan: ActionPlan {
                mode: PlanMode::Sequence,
                steps: vec![Step {
                    delay_ms: 10,
                    action: ActionSpec::new("overlayAlert", json!({"text": "hola"})),
                }],
            },
            priority: Some(3),
            ttl_ms: 1000,
            cost_points: None,
        };
        let back: Rule = serde_json::from_str(&serde_json::to_string(&rule).expect("ser")).expect("de");
        assert_eq!(back, rule);
    }

    #[test]
    fn unknown_trigger_type_is_rejected() {
        let r: Result<Rule, _> = serde_json::from_value(json!({
            "id": "r", "name": "n", "trigger": {"type": "nope"}, "plan": {"steps": []}
        }));
        assert!(r.is_err());
    }
}
