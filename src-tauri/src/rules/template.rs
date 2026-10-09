//! Plantillas de texto con variables: `Gracias {nickname} por {count}× {gift}`.

use std::collections::HashMap;

use crate::events::{EventType, LiveEvent};

pub type Vars = HashMap<String, String>;

/// Sustituye `{nombre}` por su valor. Las variables desconocidas se dejan tal cual,
/// lo que facilita detectar un error de escritura en la plantilla. No es recursivo.
pub fn render(template: &str, vars: &Vars) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if is_var_name(&after[..end]) => {
                let name = &after[..end];
                match vars.get(&name.to_ascii_lowercase()) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_var_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Variables disponibles para un evento en vivo.
pub fn event_vars(ev: &LiveEvent) -> Vars {
    let mut v = Vars::new();
    v.insert("user".into(), ev.user.unique_id.clone());
    v.insert("uniqueid".into(), ev.user.unique_id.clone());
    v.insert(
        "nickname".into(),
        if ev.user.nickname.is_empty() {
            ev.user.unique_id.clone()
        } else {
            ev.user.nickname.clone()
        },
    );
    v.insert("avatar".into(), ev.user.avatar.clone());
    v.insert("userid".into(), ev.user.id.clone());
    v.insert("event".into(), type_name(ev.kind).into());
    v.insert("ismoderator".into(), ev.user.is_moderator.to_string());
    v.insert("issubscriber".into(), ev.user.is_subscriber.to_string());
    v.insert("isfollower".into(), ev.user.is_follower.to_string());
    if let Some(l) = ev.user.gifter_level {
        v.insert("gifterlevel".into(), l.to_string());
    }
    if let Some(l) = ev.user.team_level {
        v.insert("teamlevel".into(), l.to_string());
    }
    if let Some(g) = &ev.gift {
        v.insert("gift".into(), g.name.clone());
        v.insert("coins".into(), g.coins.to_string());
        v.insert("count".into(), g.count.to_string());
        v.insert("giftimage".into(), g.image.clone());
    }
    if let Some(c) = &ev.chat {
        v.insert("text".into(), c.text.clone());
    }
    if let Some(l) = &ev.like {
        v.insert("count".into(), l.count.to_string());
        v.insert("likes".into(), l.total.to_string());
    }
    v
}

fn type_name(t: EventType) -> &'static str {
    match t {
        EventType::Gift => "gift",
        EventType::Chat => "chat",
        EventType::Like => "like",
        EventType::Follow => "follow",
        EventType::Share => "share",
        EventType::Subscribe => "subscribe",
        EventType::Join => "join",
        EventType::Emote => "emote",
        EventType::LiveEnd => "liveEnd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing::sample_event;
    use crate::events::Gift;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn replaces_known_variables() {
        let v = vars(&[("nickname", "Ana"), ("count", "5")]);
        assert_eq!(render("¡Gracias {nickname} x{count}!", &v), "¡Gracias Ana x5!");
    }

    #[test]
    fn variable_names_are_case_insensitive() {
        assert_eq!(render("{NickName}", &vars(&[("nickname", "Ana")])), "Ana");
    }

    #[test]
    fn unknown_variables_are_left_untouched() {
        assert_eq!(render("hola {nadie}", &Vars::new()), "hola {nadie}");
    }

    #[test]
    fn stray_braces_do_not_break_rendering() {
        let v = vars(&[("a", "1")]);
        assert_eq!(render("{ {a} }", &v), "{ 1 }");
        assert_eq!(render("abc {", &v), "abc {");
        assert_eq!(render("{}", &v), "{}");
        assert_eq!(render("{a b}", &v), "{a b}");
        assert_eq!(render("}{a}{", &v), "}1{");
    }

    #[test]
    fn substitution_is_not_recursive() {
        let v = vars(&[("text", "{user}"), ("user", "ana")]);
        assert_eq!(render("{text}", &v), "{user}");
    }

    #[test]
    fn unicode_is_preserved() {
        let v = vars(&[("nickname", "Ñandú🔥")]);
        assert_eq!(render("¡Hola, {nickname}! ✨", &v), "¡Hola, Ñandú🔥! ✨");
    }

    #[test]
    fn event_vars_expose_gift_data() {
        let mut ev = sample_event("1");
        ev.gift = Some(Gift {
            id: 1,
            name: "Rose".into(),
            coins: 30,
            count: 3,
            streakable: true,
            image: "img".into(),
        });
        let v = event_vars(&ev);
        assert_eq!(render("{nickname} {count}× {gift} = {coins}", &v), "Ana 3× Rose = 30");
    }

    #[test]
    fn nickname_falls_back_to_unique_id() {
        let mut ev = sample_event("1");
        ev.user.nickname = String::new();
        assert_eq!(event_vars(&ev)["nickname"], "ana");
    }
}
