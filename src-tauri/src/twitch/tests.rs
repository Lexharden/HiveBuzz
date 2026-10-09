use super::*;

#[test]
fn channels_are_normalized() {
    for (raw, want) in [("shroud", "shroud"), ("  #Shroud ", "shroud"), ("@Ninja_99", "ninja_99"), ("https://www.twitch.tv/Pokimane?x=1", "pokimane"), ("twitch.tv/abc/videos", "abc")] {
        assert_eq!(normalize_channel(raw).unwrap(), want, "{raw}");
    }
    for bad in ["", "ab", "a b c", "x".repeat(26).as_str(), "nombre-con-guion", "ñandú", "<script>", "https://twitch.tv/", "#"] {
        assert!(normalize_channel(bad).is_err(), "{bad:?} debía fallar");
    }
}

#[test]
fn seen_remembers_recent_ids_and_forgets_old_ones() {
    let mut s = Seen::new(3);
    assert!(s.insert("a"));
    assert!(!s.insert("a"));
    assert!(s.insert("b") && s.insert("c") && s.insert("d"));
    assert!(s.insert("a"), "«a» salió de la memoria al llenarse");
    assert!(!s.insert("d"));
}

#[test]
fn the_builtin_client_id_is_used_unless_the_user_sets_their_own() {
    assert_eq!(effective_client_id("", "integrado"), "integrado");
    assert_eq!(effective_client_id("  ", "integrado"), "integrado");
    assert_eq!(effective_client_id(" propio ", "integrado"), "propio");
}
