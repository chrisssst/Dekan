use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Portuguese,
    Spanish,
    English,
}

impl Language {
    #[must_use]
    pub fn from_locale(locale: &str) -> Option<Self> {
        let language = locale.split(['_', '-']).next()?.to_ascii_lowercase();
        match language.as_str() {
            "pt" => Some(Self::Portuguese),
            "es" => Some(Self::Spanish),
            "en" => Some(Self::English),
            _ => None,
        }
    }

    #[must_use]
    pub fn of_windows() -> Self {
        static CACHED: OnceLock<Language> = OnceLock::new();
        *CACHED.get_or_init(|| {
            let lang_id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
            Self::from_primary_lang_id(lang_id & 0x3ff)
        })
    }

    fn from_primary_lang_id(primary: u16) -> Self {
        match primary {
            0x16 => Self::Portuguese,
            0x0a => Self::Spanish,
            _ => Self::English,
        }
    }

    #[must_use]
    pub fn for_locale(locale: Option<&str>) -> Self {
        locale
            .and_then(Self::from_locale)
            .unwrap_or_else(Self::of_windows)
    }

    #[must_use]
    pub fn text(self) -> &'static Text {
        match self {
            Self::Portuguese => &PORTUGUESE,
            Self::Spanish => &SPANISH,
            Self::English => &ENGLISH,
        }
    }
}

static ACTIVE_LANGUAGE: std::sync::RwLock<Option<Language>> = std::sync::RwLock::new(None);

/// Set the active language from a client locale (e.g. `pt_BR`, `es_ES`, `en_US`).
pub fn set_active_locale(locale: &str) {
    if let Some(lang) = Language::from_locale(locale) {
        if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
            *lock = Some(lang);
        }
    }
}

/// Reset the active language back to Windows default.
pub fn reset_active_language() {
    if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
        *lock = None;
    }
}

/// The active language: client locale if detected, otherwise Windows display language.
#[must_use]
pub fn active_language() -> Language {
    if let Ok(lock) = ACTIVE_LANGUAGE.read() {
        if let Some(lang) = *lock {
            return lang;
        }
    }
    Language::of_windows()
}

/// The active dictionary for Dekan.
#[must_use]
pub fn text() -> &'static Text {
    active_language().text()
}

/// Every string, one field each. `{n}`, `{code}`, `{reason}` and `{error}` are filled by [`fill`].
#[derive(Debug)]
pub struct Text {
    pub status_tools_missing: &'static str,
    pub status_waiting_league: &'static str,
    pub status_connected: &'static str,
    pub status_lobby: &'static str,
    pub status_matchmaking: &'static str,
    pub status_ready_check: &'static str,
    pub status_champ_select: &'static str,
    pub status_finalization: &'static str,
    pub status_injecting: &'static str,
    pub status_in_game: &'static str,
    pub status_in_game_confirmed: &'static str,
    pub status_in_game_unconfirmed: &'static str,
    pub status_in_game_failed: &'static str,
    pub status_reconnecting: &'static str,

    pub party_off: &'static str,
    pub party_unavailable: &'static str,
    pub party_connecting: &'static str,
    pub party_in_room: &'static str,
    pub party_reconnecting: &'static str,

    pub menu_party_create: &'static str,
    pub menu_party_join: &'static str,
    pub menu_party_leave: &'static str,
    pub menu_open_mods: &'static str,
    pub menu_open_logs: &'static str,
    pub menu_open_tools: &'static str,
    pub menu_about: &'static str,
    pub menu_autostart: &'static str,
    pub menu_auto_accept: &'static str,
    pub menu_quit: &'static str,

    pub already_running_title: &'static str,
    pub already_running_body: &'static str,

    pub party_unavailable_title: &'static str,
    pub party_unavailable_body: &'static str,
    pub party_created_title: &'static str,
    pub party_created_body: &'static str,
    pub party_copy_failed_body: &'static str,
    pub party_join_title: &'static str,
    pub party_join_empty_clipboard: &'static str,
    pub party_join_clipboard_error: &'static str,
    pub party_joining: &'static str,
    pub party_invalid_code: &'static str,

    pub party_dialog_create_title: &'static str,
    pub party_dialog_create_desc: &'static str,
    pub party_dialog_join_title: &'static str,
    pub party_dialog_join_desc: &'static str,
    pub party_dialog_label_code: &'static str,
    pub party_dialog_placeholder: &'static str,
    pub party_dialog_btn_copy: &'static str,
    pub party_dialog_btn_paste: &'static str,
    pub party_dialog_btn_ok: &'static str,
    pub party_dialog_btn_join: &'static str,
    pub party_dialog_btn_cancel: &'static str,
    pub party_dialog_copied: &'static str,
    pub party_dialog_error_empty: &'static str,

    pub import_title: &'static str,
    pub import_refused: &'static str,
    pub import_unsupported_extension: &'static str,
    pub import_not_a_mod: &'static str,
    pub import_no_manifest: &'static str,
    pub import_no_content: &'static str,
    pub import_no_champion: &'static str,
    pub import_io_error: &'static str,

    pub html_lang: &'static str,
    pub welcome_active: &'static str,
    pub welcome_background: &'static str,
    pub welcome_author: &'static str,
    pub welcome_tray_hint: &'static str,
    pub welcome_dismiss: &'static str,

    pub welcome_quote: &'static str,
    /// The relay refused us because the room already holds its maximum of members.
    pub party_room_full: &'static str,

    pub about_title: &'static str,
    pub about_educational: &'static str,
    pub about_quote: &'static str,
    pub about_dismiss: &'static str,
}

#[must_use]
pub fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(&format!("{{{key}}}"), value)
}

static PORTUGUESE: Text = Text {
    status_tools_missing: "Ferramentas ausentes (injeção desativada)",
    status_waiting_league: "Aguardando o League",
    status_connected: "Conectado ao League",
    status_lobby: "No lobby",
    status_matchmaking: "Buscando partida",
    status_ready_check: "Partida encontrada",
    status_champ_select: "Seleção de campeões",
    status_finalization: "Finalizando a seleção",
    status_injecting: "Injetando a skin…",
    status_in_game: "Em jogo",
    status_in_game_confirmed: "Em jogo — skin ativa",
    status_in_game_unconfirmed: "Em jogo — skin NÃO confirmada",
    status_in_game_failed: "Em jogo — falha na injeção",
    status_reconnecting: "Reconectando",

    party_off: "Party: desligado",
    party_unavailable: "Party: indisponível (relay não configurado)",
    party_connecting: "Party: conectando…",
    party_in_room: "Party: na sala ({n} no total)",
    party_reconnecting: "Party: reconectando…",

    menu_party_create: "Criar sala de party...",
    menu_party_join: "Entrar na sala de party...",
    menu_party_leave: "Sair da party",
    menu_open_mods: "Abrir pasta de mods",
    menu_open_logs: "Abrir pasta de logs",
    menu_open_tools: "Abrir pasta de ferramentas",
    menu_about: "Sobre o Dekan...",
    menu_autostart: "Iniciar com o Windows",
    menu_auto_accept: "Aceitar partida automaticamente",
    menu_quit: "Sair do Dekan",

    already_running_title: "Dekan já está aberto",
    already_running_body: "O Dekan já está em execução em segundo plano.\n\nProcure o ícone do Dekan na \
                           bandeja do Windows.\nPara encerrá-lo, clique com o botão direito no ícone e \
                           escolha \"Sair do Dekan\".",

    party_unavailable_title: "Party indisponível",
    party_unavailable_body: "O party precisa de um relay configurado.\n\n{reason}",
    party_created_title: "Sala de party criada",
    party_created_body: "O código da sala foi copiado. Cole para seus amigos — ele vale por 1 hora.\n\n\
                         Quem tiver o código vê a skin que você escolher.",
    party_copy_failed_body: "Não foi possível copiar o código. Copie manualmente:\n\n{code}",
    party_join_title: "Entrar na party",
    party_join_empty_clipboard: "Copie o código da sala que seu amigo enviou e tente de novo.",
    party_join_clipboard_error: "A área de transferência não pôde ser lida: {error}",
    party_joining: "Entrando na sala. O status aparece no menu da bandeja.",
    party_invalid_code: "Código de party inválido: {error}",

    party_dialog_create_title: "Sala de Party Criada",
    party_dialog_create_desc: "Envie este código para seus amigos no mesmo time para verem suas skins:",
    party_dialog_join_title: "Entrar na Sala de Party",
    party_dialog_join_desc: "Insira ou cole o código da sala de party enviado pelo seu amigo:",
    party_dialog_label_code: "Código da Sala (Party)",
    party_dialog_placeholder: "Cole o código aqui (DEKAN1:...)",
    party_dialog_btn_copy: "Copiar Código",
    party_dialog_btn_paste: "Colar",
    party_dialog_btn_ok: "Concluir",
    party_dialog_btn_join: "Entrar na Sala",
    party_dialog_btn_cancel: "Cancelar",
    party_dialog_copied: "Copiado! ✓",
    party_dialog_error_empty: "Por favor, insira o código da sala.",

    import_title: "Dekan — importar mod",
    import_refused: "O arquivo não foi importado:\n{reason}",
    import_unsupported_extension: "só é possível importar mods .fantome e .zip",
    import_not_a_mod: "não é um pacote de mod ({error})",
    import_no_manifest: "falta o manifesto META/info.json",
    import_no_content: "o pacote não tem conteúdo em WAD/ nem em RAW/",
    import_no_champion: "o pacote não diz de qual campeão é: escolha o campeão na seleção e importe de novo",
    import_io_error: "erro ao gravar o mod: {error}",

    html_lang: "pt-BR",
    welcome_active: "ATIVO NA BANDEJA DO SISTEMA",
    welcome_background: "O Dekan permanece minimizado em segundo plano",
    welcome_author: "Build personalizado do Dekan.",
    welcome_tray_hint: "Clique com o botão direito no ícone da bandeja para abrir mods, logs ou gerenciar a party.",
    welcome_dismiss: "ENTENDIDO",
    welcome_quote: "“Eu sempre atiro primeiro.” — Miss Fortune",
    party_room_full: "A sala de party está cheia. Peça para alguém sair e tente entrar de novo.",

    about_title: "SOBRE O DEKAN",
    about_educational: "Projeto educacional e sem fins lucrativos, para estudo de engenharia reversa, formatos de arquivo do jogo e injeção no Windows. Use por sua conta e risco: alterar o cliente viola os Termos de Serviço da Riot Games e pode levar a banimento. Dekan não é afiliado à Riot Games.",
    about_quote: "\u{201c}Eu sempre atiro primeiro.\u{201d} \u{2014} Miss Fortune",
    about_dismiss: "FECHAR",
};

static SPANISH: Text = Text {
    status_tools_missing: "Faltan las herramientas (inyección desactivada)",
    status_waiting_league: "Esperando a League",
    status_connected: "Conectado a League",
    status_lobby: "En la sala",
    status_matchmaking: "Buscando partida",
    status_ready_check: "Partida encontrada",
    status_champ_select: "Selección de campeones",
    status_finalization: "Finalizando la selección",
    status_injecting: "Inyectando el aspecto…",
    status_in_game: "En partida",
    status_in_game_confirmed: "En partida — aspecto activo",
    status_in_game_unconfirmed: "En partida — aspecto NO confirmado",
    status_in_game_failed: "En partida — fallo en la inyección",
    status_reconnecting: "Reconectando",

    party_off: "Party: desactivado",
    party_unavailable: "Party: no disponible (relay sin configurar)",
    party_connecting: "Party: conectando…",
    party_in_room: "Party: en la sala ({n} en total)",
    party_reconnecting: "Party: reconectando…",

    menu_party_create: "Crear sala de party...",
    menu_party_join: "Unirse a la sala de party...",
    menu_party_leave: "Salir de la party",
    menu_open_mods: "Abrir carpeta de mods",
    menu_open_logs: "Abrir carpeta de registros",
    menu_open_tools: "Abrir carpeta de herramientas",
    menu_about: "Acerca de Dekan...",
    menu_autostart: "Iniciar con Windows",
    menu_auto_accept: "Aceptar partida automáticamente",
    menu_quit: "Salir de Dekan",

    already_running_title: "Dekan ya está abierto",
    already_running_body: "Dekan ya se está ejecutando en segundo plano.\n\nBusca el icono de Dekan en la \
                           bandeja de Windows.\nPara cerrarlo, haz clic derecho en el icono y elige \
                           \"Salir de Dekan\".",

    party_unavailable_title: "Party no disponible",
    party_unavailable_body: "La party necesita un relay configurado.\n\n{reason}",
    party_created_title: "Sala de party creada",
    party_created_body: "Se copió el código de la sala. Pégalo a tus amigos — vale por 1 hora.\n\n\
                         Quien tenga el código verá el aspecto que elijas.",
    party_copy_failed_body: "No se pudo copiar el código. Cópialo a mano:\n\n{code}",
    party_join_title: "Unirse a la party",
    party_join_empty_clipboard: "Copia el código de la sala que te envió tu amigo e inténtalo de nuevo.",
    party_join_clipboard_error: "No se pudo leer el portapapeles: {error}",
    party_joining: "Entrando en la sala. El estado aparece en el menú de la bandeja.",
    party_invalid_code: "Código de party no válido: {error}",

    party_dialog_create_title: "Sala de Party Creada",
    party_dialog_create_desc: "Envía este código a tus amigos en el mismo equipo para sincronizar aspectos:",
    party_dialog_join_title: "Unirse a la Sala de Party",
    party_dialog_join_desc: "Introduce o pega el código de sala que te envió tu amigo:",
    party_dialog_label_code: "Código de Sala (Party)",
    party_dialog_placeholder: "Pega el código aquí (DEKAN1:...)",
    party_dialog_btn_copy: "Copiar Código",
    party_dialog_btn_paste: "Pegar",
    party_dialog_btn_ok: "Aceptar",
    party_dialog_btn_join: "Entrar a la Sala",
    party_dialog_btn_cancel: "Cancelar",
    party_dialog_copied: "¡Copiado! ✓",
    party_dialog_error_empty: "Por favor, introduce el código de la sala.",

    import_title: "Dekan — importar mod",
    import_refused: "El archivo no se importó:\n{reason}",
    import_unsupported_extension: "solo se pueden importar mods .fantome y .zip",
    import_not_a_mod: "no es un paquete de mod ({error})",
    import_no_manifest: "falta el manifiesto META/info.json",
    import_no_content: "el paquete no tiene contenido en WAD/ ni en RAW/",
    import_no_champion: "el paquete no indica de qué campeón es: elige el campeón en la selección e impórtalo de nuevo",
    import_io_error: "error al guardar el mod: {error}",

    html_lang: "es",
    welcome_active: "ACTIVO EN LA BANDEJA DEL SISTEMA",
    welcome_background: "Dekan sigue minimizado en segundo plano",
    welcome_author: "Compilación personalizada de Dekan.",
    welcome_tray_hint: "Haz clic derecho en el icono de la bandeja para abrir mods, logs o gestionar la party.",
    welcome_dismiss: "ENTENDIDO",
    welcome_quote: "",
    party_room_full: "La sala de party está llena. Pide que alguien salga e intenta entrar de nuevo.",

    about_title: "ACERCA DE DEKAN",
    about_educational: "Proyecto educativo y sin fines de lucro, para el estudio de ingeniería inversa, formatos de archivo del juego e inyección en Windows. Úsalo bajo tu propio riesgo: modificar el cliente infringe los Términos de Servicio de Riot Games y puede provocar un baneo. Dekan no está afiliado a Riot Games.",
    about_quote: "\u{201c}Siempre disparo primero.\u{201d} \u{2014} Miss Fortune",
    about_dismiss: "CERRAR",
};

static ENGLISH: Text = Text {
    status_tools_missing: "Tools missing (injection disabled)",
    status_waiting_league: "Waiting for League",
    status_connected: "Connected to League",
    status_lobby: "In lobby",
    status_matchmaking: "Finding a match",
    status_ready_check: "Match found",
    status_champ_select: "Champion select",
    status_finalization: "Finalizing selection",
    status_injecting: "Injecting the skin…",
    status_in_game: "In game",
    status_in_game_confirmed: "In game — skin active",
    status_in_game_unconfirmed: "In game — skin NOT confirmed",
    status_in_game_failed: "In game — injection failed",
    status_reconnecting: "Reconnecting",

    party_off: "Party: off",
    party_unavailable: "Party: unavailable (no relay configured)",
    party_connecting: "Party: connecting…",
    party_in_room: "Party: in the room ({n} in total)",
    party_reconnecting: "Party: reconnecting…",

    menu_party_create: "Create party room...",
    menu_party_join: "Join party room...",
    menu_party_leave: "Leave party",
    menu_open_mods: "Open mods folder",
    menu_open_logs: "Open logs folder",
    menu_open_tools: "Open tools folder",
    menu_about: "About Dekan...",
    menu_autostart: "Start with Windows",
    menu_auto_accept: "Accept matches automatically",
    menu_quit: "Quit Dekan",

    already_running_title: "Dekan is already open",
    already_running_body: "Dekan is already running in the background.\n\nLook for the Dekan icon in the \
                           Windows tray.\nTo close it, right-click the icon and choose \"Quit Dekan\".",

    party_unavailable_title: "Party unavailable",
    party_unavailable_body: "Party mode needs a configured relay.\n\n{reason}",
    party_created_title: "Party room created",
    party_created_body: "The room code was copied. Paste it to your friends — it is valid for 1 hour.\n\n\
                         Whoever has the code sees the skin you pick.",
    party_copy_failed_body: "The code could not be copied. Copy it by hand:\n\n{code}",
    party_join_title: "Join party",
    party_join_empty_clipboard: "Copy the room code your friend sent and try again.",
    party_join_clipboard_error: "The clipboard could not be read: {error}",
    party_joining: "Joining the room. The status shows in the tray menu.",
    party_invalid_code: "Invalid party code: {error}",

    party_dialog_create_title: "Party Room Created",
    party_dialog_create_desc: "Send this code to your teammates so they see your custom skins:",
    party_dialog_join_title: "Join Party Room",
    party_dialog_join_desc: "Enter or paste the room code sent by your friend:",
    party_dialog_label_code: "Room Code (Party)",
    party_dialog_placeholder: "Paste room code here (DEKAN1:...)",
    party_dialog_btn_copy: "Copy Code",
    party_dialog_btn_paste: "Paste",
    party_dialog_btn_ok: "Done",
    party_dialog_btn_join: "Join Room",
    party_dialog_btn_cancel: "Cancel",
    party_dialog_copied: "Copied! ✓",
    party_dialog_error_empty: "Please enter the room code.",

    import_title: "Dekan — import mod",
    import_refused: "The file was not imported:\n{reason}",
    import_unsupported_extension: "only .fantome and .zip mods can be imported",
    import_not_a_mod: "not a mod package ({error})",
    import_no_manifest: "the META/info.json manifest is missing",
    import_no_content: "the package has no WAD/ or RAW/ content",
    import_no_champion: "the package does not say which champion it is for: pick the champion in champ select and import it again",
    import_io_error: "could not write the mod: {error}",

    html_lang: "en",
    welcome_active: "ACTIVE IN THE SYSTEM TRAY",
    welcome_background: "Dekan stays minimized in the background",
    welcome_author: "Dekan custom build.",
    welcome_tray_hint: "Right-click the tray icon to open your mods folder, manage party rooms, or inspect logs.",
    welcome_dismiss: "GOT IT",
    welcome_quote: "",
    party_room_full: "The party room is full. Ask someone to leave and try joining again.",

    about_title: "ABOUT DEKAN",
    about_educational: "An educational, non-commercial project for studying reverse engineering, the game's file formats and Windows injection. Use at your own risk: modifying the client violates Riot Games' Terms of Service and may lead to a ban. Dekan is not affiliated with Riot Games.",
    about_quote: "\u{201c}I always shoot first.\u{201d} \u{2014} Miss Fortune",
    about_dismiss: "CLOSE",
};

#[cfg(test)]
#[path = "i18n_tests.rs"]
mod tests;
