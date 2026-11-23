use serde::{Deserialize, Serialize};

const MAX_PATH_LEN: usize = 200;
const MAX_TITLE_LEN: usize = 64;
const MAX_BORDER_LEN: usize = 48;
const MAX_TOKENS: usize = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdentityLoadout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tokens: Vec<String>,
}

fn valid_asset_path(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_PATH_LEN || value.contains("..") {
        return false;
    }
    let allowed_prefix = value.starts_with("/lol-game-data/assets/")
        || value.starts_with("/fe/lol-static-assets/")
        || value.starts_with("fe/lol-static-assets/");
    if !allowed_prefix {
        return false;
    }
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
}

fn valid_border(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_BORDER_LEN {
        return false;
    }
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
}

fn valid_title(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_TITLE_LEN {
        return false;
    }
    !trimmed
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '\\'))
}

fn valid_token(value: &str) -> bool {
    if valid_asset_path(value) {
        return true;
    }
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
}

impl IdentityLoadout {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.icon_id.is_none()
            && self.border.is_none()
            && self.banner.is_none()
            && self.title.is_none()
            && self.tokens.is_empty()
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(icon) = self.icon_id
            && icon == 0
        {
            return Err("icon_id");
        }
        if let Some(border) = self.border.as_deref()
            && !valid_border(border)
        {
            return Err("border");
        }
        if let Some(banner) = self.banner.as_deref()
            && !valid_asset_path(banner)
        {
            return Err("banner");
        }
        if let Some(title) = self.title.as_deref()
            && !valid_title(title)
        {
            return Err("title");
        }
        if self.tokens.len() > MAX_TOKENS {
            return Err("tokens count");
        }
        for token in &self.tokens {
            if !valid_token(token) {
                return Err("token");
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn normalized(mut self) -> Option<Self> {
        self.icon_id = self.icon_id.filter(|&id| id > 0);
        self.border = self.border.filter(|b| valid_border(b));
        self.banner = self.banner.filter(|b| valid_asset_path(b));
        self.title = self
            .title
            .map(|t| t.trim().to_owned())
            .filter(|t| valid_title(t));
        self.tokens.retain(|t| valid_token(t));
        self.tokens.truncate(MAX_TOKENS);
        (!self.is_empty()).then_some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_identity_passes_and_rejects_css_injection() {
        let ok = IdentityLoadout {
            icon_id: Some(29),
            border: Some("CHALLENGER".into()),
            banner: Some("/lol-game-data/assets/v1/regalia/banner_challenger.png".into()),
            title: Some("Bot Diff".into()),
            tokens: vec![
                "/lol-game-data/assets/assets/challenges/config/101000/tokens/challenger.png"
                    .into(),
                "101000:CHALLENGER".into(),
            ],
        };
        assert!(ok.validate().is_ok());
        assert!(!ok.is_empty());

        let css_inj = IdentityLoadout {
            banner: Some("/lol-game-data/assets/x.png);background:url(evil".into()),
            ..IdentityLoadout::default()
        };
        assert!(css_inj.validate().is_err());

        let traversal = IdentityLoadout {
            banner: Some("/lol-game-data/assets/../secrets.txt".into()),
            ..IdentityLoadout::default()
        };
        assert!(traversal.validate().is_err());

        let xss_title = IdentityLoadout {
            title: Some("<script>alert(1)</script>".into()),
            ..IdentityLoadout::default()
        };
        assert!(xss_title.validate().is_err());

        let too_many_tokens = IdentityLoadout {
            tokens: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            ..IdentityLoadout::default()
        };
        assert!(too_many_tokens.validate().is_err());
    }
}
