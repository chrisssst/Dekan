const PROFILE_MARK: &str = "%USERPROFILE%";
const UUID_MARK: &str = "<uuid>";
const USER_MARK: &str = "<user>";
const USERS_DIRS: [&str; 3] = [r"\\users\\", r"\users\", "/users/"];
const UUID_GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
const UUID_LEN: usize = 36;

#[must_use]
pub(super) fn redact(text: &str, profile: Option<&str>) -> String {
    let text = match profile.filter(|p| p.len() > 3) {
        Some(profile) => profile_variants(profile)
            .iter()
            .fold(text.to_owned(), |acc, variant| {
                replace_ignore_ascii_case(&acc, variant, PROFILE_MARK)
            }),
        None => text.to_owned(),
    };
    mask_uuids(&mask_user_folders(&text))
}

fn mask_user_folders(text: &str) -> String {
    USERS_DIRS.iter().fold(text.to_owned(), |text, dir| {
        let haystack = text.to_ascii_lowercase();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (at, _) in haystack.match_indices(dir) {
            let name = at + dir.len();
            if at < last {
                continue;
            }
            let end = text[name..]
                .find(['\\', '/', '"', '\n'])
                .map_or(text.len(), |n| name + n);
            let folder = &text[name..end];
            if folder.is_empty() || folder.starts_with('%') || folder == USER_MARK {
                continue;
            }
            out.push_str(&text[last..name]);
            out.push_str(USER_MARK);
            last = end;
        }
        out.push_str(&text[last..]);
        out
    })
}

fn profile_variants(profile: &str) -> [String; 3] {
    let profile = profile.trim_end_matches(['\\', '/']);
    [
        profile.replace('\\', "\\\\"),
        profile.to_owned(),
        profile.replace('\\', "/"),
    ]
}

fn replace_ignore_ascii_case(text: &str, needle: &str, with: &str) -> String {
    let haystack = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (at, _) in haystack.match_indices(&needle) {
        out.push_str(&text[last..at]);
        out.push_str(with);
        last = at + needle.len();
    }
    out.push_str(&text[last..]);
    out
}

fn mask_uuids(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut at = 0;
    while at + UUID_LEN <= bytes.len() {
        let bounded = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let ends = bytes
            .get(at + UUID_LEN)
            .is_none_or(|b| !b.is_ascii_alphanumeric());
        if bounded && ends && is_uuid(&bytes[at..at + UUID_LEN]) {
            out.push_str(&text[last..at]);
            out.push_str(UUID_MARK);
            at += UUID_LEN;
            last = at;
        } else {
            at += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

fn is_uuid(candidate: &[u8]) -> bool {
    let mut groups = candidate.split(|b| *b == b'-');
    UUID_GROUPS.iter().all(|len| {
        groups
            .next()
            .is_some_and(|g| g.len() == *len && g.iter().all(u8::is_ascii_hexdigit))
    }) && groups.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::redact;

    const PROFILE: &str = r"C:\Users\Ana Silva";

    #[test]
    fn the_profile_is_masked_in_every_spelling() {
        let log = r#"dir=C:\Users\Ana Silva\AppData json="c:\\users\\ana silva\\x" url=C:/Users/Ana Silva/y"#;
        assert_eq!(
            redact(log, Some(PROFILE)),
            r#"dir=%USERPROFILE%\AppData json="%USERPROFILE%\\x" url=%USERPROFILE%/y"#
        );
    }

    #[test]
    fn uuids_are_masked_only_when_whole() {
        let log =
            "puuid=0f8fad5b-d9cb-469f-a165-70867728950e x0f8fad5b-d9cb-469f-a165-70867728950e";
        assert_eq!(
            redact(log, None),
            "puuid=<uuid> x0f8fad5b-d9cb-469f-a165-70867728950e"
        );
        assert_eq!(
            redact("0f8fad5b-d9cb-469f-a165-7086772895", None),
            "0f8fad5b-d9cb-469f-a165-7086772895"
        );
    }

    #[test]
    fn any_user_folder_is_masked_including_short_names() {
        let log = r#"tmp=C:\Users\DEKAN~1\AppData x="d:\\users\\other\\y" z=E:/Users/Bia Lima/w"#;
        assert_eq!(
            redact(log, Some(PROFILE)),
            r#"tmp=C:\Users\<user>\AppData x="d:\\users\\<user>\\y" z=E:/Users/<user>/w"#
        );
        assert_eq!(
            redact(r"dir=C:\Users\Ana Silva\AppData", Some(PROFILE)),
            r"dir=%USERPROFILE%\AppData"
        );
    }

    #[test]
    fn text_without_private_data_is_unchanged() {
        let log = "Game WAD index ready wads=812 elapsed_ms=461 é";
        assert_eq!(redact(log, Some(PROFILE)), log);
        assert_eq!(redact(log, Some("C:")), log);
    }
}
