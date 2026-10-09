use super::*;

fn parse_ok(line: &str) -> IrcMsg {
    parse(line).unwrap_or_else(|| panic!("no se pudo analizar: {line}"))
}

const CHAT: &str = "@badge-info=subscriber/8;badges=moderator/1,subscriber/6;color=#FF0000;display-name=Ana\\sB;emotes=25:0-4,12-16/1902:6-10;id=msg-1;user-id=12345;tmi-sent-ts=1700000000123 :ana!ana@ana.tmi.twitch.tv PRIVMSG #canal :Kappa hola Kappa";

#[test]
fn parses_tags_prefix_command_and_trailing() {
    let m = parse_ok(CHAT);
    assert_eq!(m.command, "PRIVMSG");
    assert_eq!(m.params, ["#canal"]);
    assert_eq!(m.trailing.as_deref(), Some("Kappa hola Kappa"));
    assert_eq!(m.prefix, "ana!ana@ana.tmi.twitch.tv");
    assert_eq!(m.tags["display-name"], "Ana B", r"\s se convierte en espacio");
    assert_eq!(m.tags["id"], "msg-1");
}

#[test]
fn parses_lines_without_tags_or_trailing() {
    let ping = parse_ok("PING :tmi.twitch.tv");
    assert_eq!((ping.command.as_str(), ping.trailing.as_deref()), ("PING", Some("tmi.twitch.tv")));
    let join = parse_ok(":tmi.twitch.tv 366 justinfan1 #canal :End of /NAMES list");
    assert_eq!(join.command, "366");
    let roomstate = parse_ok("@room-id=1;slow=0 :tmi.twitch.tv ROOMSTATE #canal");
    assert_eq!((roomstate.command.as_str(), roomstate.trailing.as_deref()), ("ROOMSTATE", None));
}

#[test]
fn tag_escapes_are_decoded() {
    let m = parse_ok(r"@a=x\:y\\z\r\n;b= :s CMD");
    assert_eq!(m.tags["a"], "x;y\\z\r\n");
    assert_eq!(m.tags["b"], "");
}

#[test]
fn malformed_lines_never_panic() {
    for bad in ["", "   ", "@", "@tags", ":", ":prefix", "@a=b", "\r\n"] {
        let _ = parse(bad);
    }
    assert!(parse("").is_none() && parse("   ").is_none());
    let long = format!("@a={} :x PRIVMSG #c :{}", "ñ".repeat(5000), "z".repeat(10_000));
    let _ = parse(&long);
}

#[test]
fn a_chat_line_becomes_a_chat_event_with_roles_emotes_and_prefixed_id() {
    let evs = to_events(&parse_ok(CHAT), 1);
    assert_eq!(evs.len(), 1);
    let e = &evs[0];
    assert_eq!((e.platform, e.kind, e.id.as_str(), e.ts), (Platform::Twitch, EventType::Chat, "msg-1", 1_700_000_000_123));
    assert_eq!((e.user.id.as_str(), e.user.unique_id.as_str(), e.user.nickname.as_str()), ("tw:12345", "ana", "Ana B"));
    assert!(e.user.is_moderator && e.user.is_subscriber && !e.user.is_follower);
    let chat = e.chat.as_ref().unwrap();
    assert_eq!(chat.text, "Kappa hola Kappa");
    let emotes = chat.emotes.as_ref().unwrap();
    assert_eq!(emotes.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["25", "1902"]);
    assert_eq!(emotes[0].image, "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0");
}

#[test]
fn the_broadcaster_counts_as_a_moderator() {
    let m = parse_ok("@badges=broadcaster/1;display-name=D;id=i;user-id=9 :d!d@d.tmi.twitch.tv PRIVMSG #d :hola");
    assert!(to_events(&m, 1)[0].user.is_moderator);
}

#[test]
fn action_messages_lose_their_wrapper() {
    let m = parse_ok("@id=i;user-id=9 :d!d@d.tmi.twitch.tv PRIVMSG #d :\u{1}ACTION baila\u{1}");
    assert_eq!(to_events(&m, 1)[0].chat.as_ref().unwrap().text, "baila");
}

#[test]
fn cheers_add_a_bits_gift_without_extra_scopes() {
    let m = parse_ok("@bits=100;display-name=Ana;id=m9;user-id=5 :ana!ana@ana.tmi.twitch.tv PRIVMSG #c :cheer100 gracias");
    let evs = to_events(&m, 7);
    assert_eq!(evs.len(), 2);
    assert_eq!(evs[0].kind, EventType::Chat);
    let g = &evs[1];
    assert_eq!((g.kind, g.id.as_str()), (EventType::Gift, "m9-bits"));
    let gift = g.gift.as_ref().unwrap();
    assert_eq!((gift.name.as_str(), gift.coins, gift.count, gift.streakable), ("Bits", 100, 1, false));
    assert_ne!(evs[0].id, evs[1].id, "ids distintos para deduplicar por separado");
}

#[test]
fn subscriptions_come_from_usernotice() {
    let sub = parse_ok("@badges=subscriber/0;display-name=Bea;id=n1;login=bea;msg-id=resub;user-id=77 :tmi.twitch.tv USERNOTICE #c :gracias");
    let e = &to_events(&sub, 1)[0];
    assert_eq!((e.kind, e.user.id.as_str(), e.user.unique_id.as_str()), (EventType::Subscribe, "tw:77", "bea"));
    assert!(e.user.is_subscriber);
}

#[test]
fn a_gifted_sub_subscribes_the_recipient_not_the_gifter() {
    let m = parse_ok("@badges=moderator/1;display-name=Gifter;id=n2;login=gifter;msg-id=subgift;msg-param-recipient-display-name=Cris;msg-param-recipient-id=88;msg-param-recipient-user-name=cris;user-id=1 :tmi.twitch.tv USERNOTICE #c");
    let e = &to_events(&m, 1)[0];
    assert_eq!((e.user.id.as_str(), e.user.unique_id.as_str(), e.user.nickname.as_str()), ("tw:88", "cris", "Cris"));
    assert!(!e.user.is_moderator, "las insignias del que regala no pasan al destinatario");
}

#[test]
fn mass_gifts_raids_and_unknown_notices_make_no_events() {
    for id in ["submysterygift", "raid", "ritual", "announcement", "viewermilestone"] {
        let m = parse_ok(&format!("@id=n;login=x;msg-id={id};user-id=1;display-name=X :tmi.twitch.tv USERNOTICE #c"));
        assert!(to_events(&m, 1).is_empty(), "{id}");
    }
    for cmd in ["JOIN #c", "PART #c", "CLEARCHAT #c :x", "NOTICE #c :hola"] {
        assert!(to_events(&parse_ok(&format!(":tmi.twitch.tv {cmd}")), 1).is_empty());
    }
}

#[test]
fn lines_without_a_numeric_user_id_or_message_id_are_dropped() {
    for line in [
        "@id=i;user-id=abc :d!d@d.tmi.twitch.tv PRIVMSG #d :hola",
        "@id=i :d!d@d.tmi.twitch.tv PRIVMSG #d :hola",
        "@user-id=9 :d!d@d.tmi.twitch.tv PRIVMSG #d :hola",
        "@id=i;user-id=9 :d!d@d.tmi.twitch.tv PRIVMSG #d",
    ] {
        assert!(to_events(&parse_ok(line), 1).is_empty(), "{line}");
    }
}

#[test]
fn hostile_emote_ids_are_ignored() {
    let m = parse_ok("@emotes=..%2F..%2Fx:0-1/ok_1:2-3;id=i;user-id=9 :d!d@d.tmi.twitch.tv PRIVMSG #d :ab cd");
    let emotes = to_events(&m, 1)[0].chat.as_ref().unwrap().emotes.clone().unwrap();
    assert_eq!(emotes.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["ok_1"]);
}
