use super::Language;

#[must_use]
pub fn installed_language() -> Option<Language> {
    static LANGUAGE: std::sync::OnceLock<Option<Language>> = std::sync::OnceLock::new();
    *LANGUAGE.get_or_init(|| {
        let executable = std::env::current_exe().ok()?;
        let path = executable.parent()?.join("language.ini");
        let metadata = std::fs::metadata(&path).ok()?;
        if metadata.len() > 4096 {
            return None;
        }
        parse(&std::fs::read_to_string(path).ok()?)
    })
}

fn parse(source: &str) -> Option<Language> {
    let mut ui = false;
    for line in source.lines().map(str::trim) {
        if line.starts_with('[') {
            ui = line.eq_ignore_ascii_case("[ui]");
        } else if let Some((_, value)) = line
            .split_once('=')
            .filter(|(key, _)| ui && key.trim().eq_ignore_ascii_case("language"))
        {
            return match value.trim().to_ascii_lowercase().as_str() {
                "turkish" => Some(Language::Turkish),
                "english" => Some(Language::English),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_language_is_restricted_to_the_ui_section_and_two_languages() {
        assert_eq!(
            parse("[ui]\r\nlanguage=turkish\r\n"),
            Some(Language::Turkish)
        );
        assert_eq!(parse("[ui]\nlanguage=english"), Some(Language::English));
        for source in [
            "language=english",
            "[other]\nlanguage=english",
            "[ui]\nlanguage=portuguese",
            "",
        ] {
            assert_eq!(parse(source), None);
        }
    }
}
