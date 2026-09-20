//! Source read-path ID selection, shared with the transitional native iterator.
//! Ordering, duplicate-store suppression and pure-server trust are separate.

/// No requested ID sees public paths only. An explicit ID bypasses the
/// by-request-only flag. BSP is reserved: it selects GAME map packs, never
/// standalone ZIPs, loose directories, VPKs or a literal BSP path-ID mount.
pub fn path_id_matches(
    stored: &str,
    requested: Option<&str>,
    by_request_only: bool,
    is_map_pack: bool,
) -> bool {
    match requested {
        None => !by_request_only,
        Some(id) if id.eq_ignore_ascii_case("BSP") => {
            is_map_pack && stored.eq_ignore_ascii_case("GAME")
        }
        Some(id) => stored.eq_ignore_ascii_case(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_ids_visibility_and_reserved_bsp_match_native_policy() {
        let ids = ["GAME", "game", "MOD", "BSP", "PRIVATE", "\u{e9}ID"];
        for stored in ids {
            for requested in [
                None,
                Some("GAME"),
                Some("game"),
                Some("bSp"),
                Some("MOD"),
                Some("PRIVATE"),
                Some("\u{e9}ID"),
            ] {
                for private in [false, true] {
                    for map in [false, true] {
                        let expected = if let Some(id) = requested {
                            if id.eq_ignore_ascii_case("BSP") {
                                stored.eq_ignore_ascii_case("GAME") && map
                            } else {
                                stored.eq_ignore_ascii_case(id)
                            }
                        } else {
                            !private
                        };
                        assert_eq!(path_id_matches(stored, requested, private, map), expected);
                    }
                }
            }
        }
        assert!(!path_id_matches("GAME", Some(""), false, true));
        assert!(!path_id_matches("\u{e9}ID", Some("\u{c9}ID"), false, false));
    }
}
