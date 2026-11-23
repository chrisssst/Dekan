use base64::{Engine, engine::general_purpose::STANDARD};
use dekan_core::overlay::{Catalog, OverlayCommand};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn install() -> Result<String, serde_json::Error> {
    let css = serde_json::to_string(include_str!("web/native.css"))?;
    let icon = serde_json::to_string(&format!(
        "data:image/png;base64,{}",
        STANDARD.encode(include_bytes!(
            "../../../../dekan-platform/ui/assets/logo-56.png"
        ))
    ))?;
    let source = [
        include_str!("web/i18n.js"),
        include_str!("web/core.js"),
        include_str!("web/mods.js"),
        include_str!("web/identity/identity.js"),
        include_str!("web/identity/identity-render.js"),
        include_str!("web/carousel.js"),
    ]
    .join("\n");
    Ok(format!(
        "(function() {{ const CSS = {css}; const DEKAN_ICON_URI = {icon}; {source}\n }})()"
    ))
}

pub(super) fn snapshot(catalog: Option<&Catalog>) -> Value {
    let Some(c) = catalog else { return Value::Null };
    let mut mods = serde_json::to_value(&c.mods.available).unwrap_or(Value::Null);
    mods["selection"] = json!(c.mods.selection);
    json!({
        "champion_id": c.champion_id,
        "champion_name": c.champion_name,
        "locale": c.locale,
        "skins": c.skins.iter().map(|s| json!({
            "id": s.id, "name": s.name,
            "chromas": s.chromas.iter().map(|ch| json!({
                "id": ch.id, "name": ch.name, "color": ch.color,
                "form": ch.form, "preview_path": ch.preview_path,
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "mods": mods, "party": c.party,
    })
}

pub(super) fn sync(catalog: &Value, selected: Option<u32>, language: &str) -> String {
    let lang = if language.starts_with("tr") {
        "tr"
    } else {
        "en"
    };
    format!(
        r#"(() => {{
        window.__dekanLanguage = "{lang}";
        const api = window.__dekanInClient;
        if (!api || api.version !== 18) return null;
        const commands = api.sync({catalog}, {});
        return {{commands, active: !!document.querySelector(
            '.skin-selection-carousel, .quick-play-skin-select-component, .skins-pane'
        )}};
    }})()"#,
        json!(selected)
    )
}

pub(super) const CLEANUP: &str = "window.__dekanInClient && window.__dekanInClient.cleanup()";

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Command {
    Select {
        id: u32,
    },
    Clear,
    Random,
    SetMods {
        selection: dekan_core::mods::ModSelectionView,
    },
    SetIdentity {
        loadout: Option<dekan_core::identity::IdentityLoadout>,
    },
    JoinLobbyParty,
    OpenModsFolder,
    OpenPanel,
    TogglePreset,
    ChromaPreview {
        id: u32,
    },
}

pub(super) fn commands(value: &Value) -> Vec<OverlayCommand> {
    let Some(queue) = value.as_array().filter(|q| q.len() <= 64) else {
        return Vec::new();
    };
    queue
        .iter()
        .filter_map(|v| serde_json::from_value::<Command>(v.clone()).ok())
        .map(|c| match c {
            Command::Select { id } => OverlayCommand::Select { id },
            Command::Clear => OverlayCommand::Clear,
            Command::Random => OverlayCommand::Random,
            Command::SetMods { selection } => OverlayCommand::SetMods { selection },
            Command::SetIdentity { loadout } => OverlayCommand::SetIdentity { loadout },
            Command::JoinLobbyParty => OverlayCommand::JoinLobbyParty,
            Command::OpenModsFolder => OverlayCommand::OpenModsFolder,
            Command::OpenPanel => OverlayCommand::OpenPanel,
            Command::TogglePreset => OverlayCommand::TogglePreset,
            Command::ChromaPreview { id } => OverlayCommand::ChromaPreview { id },
        })
        .collect()
}
