use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Turkish,
    English,
}

impl Language {
    #[must_use]
    pub fn from_locale(locale: &str) -> Option<Self> {
        let language = locale.split(['_', '-']).next()?.to_ascii_lowercase();
        match language.as_str() {
            "tr" => Some(Self::Turkish),
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
            0x09 => Self::English,
            _ => Self::Turkish,
        }
    }

    #[must_use]
    pub fn for_locale(locale: Option<&str>) -> Self {
        locale
            .and_then(Self::from_locale)
            .unwrap_or(Self::Turkish)
    }

    #[must_use]
    pub fn text(self) -> &'static Text {
        match self {
            Self::Turkish => &TURKISH,
            Self::English => &ENGLISH,
        }
    }
}

static ACTIVE_LANGUAGE: std::sync::RwLock<Option<Language>> = std::sync::RwLock::new(None);

pub fn set_active_locale(locale: &str) {
    if let Some(lang) = Language::from_locale(locale) {
        if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
            *lock = Some(lang);
        }
    }
}

pub fn reset_active_language() {
    if let Ok(mut lock) = ACTIVE_LANGUAGE.write() {
        *lock = None;
    }
}

#[must_use]
pub fn active_language() -> Language {
    if let Ok(lock) = ACTIVE_LANGUAGE.read() {
        if let Some(lang) = *lock {
            return lang;
        }
    }
    Language::Turkish
}

#[must_use]
pub fn text() -> &'static Text {
    active_language().text()
}

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
    pub party_created_connecting: &'static str,
    pub party_created_in_room: &'static str,

    pub menu_party_create: &'static str,
    pub menu_party_join: &'static str,
    pub menu_party_leave: &'static str,
    pub menu_group_party: &'static str,
    pub menu_group_folders: &'static str,
    pub menu_open_mods: &'static str,
    pub menu_open_logs: &'static str,
    pub menu_open_tools: &'static str,
    pub menu_about: &'static str,
    pub menu_autostart: &'static str,
    pub menu_auto_accept: &'static str,
    pub menu_quit: &'static str,
    pub menu_open_panel: &'static str,
    pub menu_random_skin: &'static str,
    pub panel_section_options: &'static str,
    pub panel_section_diagnostics: &'static str,
    pub panel_random_skin_hint: &'static str,
    pub check_injector: &'static str,
    pub check_game: &'static str,
    pub check_client: &'static str,
    pub check_dll: &'static str,
    pub check_privileges: &'static str,
    pub detail_ok: &'static str,
    pub detail_injector_missing: &'static str,
    pub detail_game_missing: &'static str,
    pub detail_client_connected: &'static str,
    pub detail_client_waiting: &'static str,
    pub detail_dll_days_left: &'static str,
    pub detail_dll_refused: &'static str,
    pub detail_dll_unknown: &'static str,
    pub detail_elevated: &'static str,
    pub detail_not_elevated: &'static str,
    pub update_available_title: &'static str,
    pub update_available_body: &'static str,
    pub panel_update_line: &'static str,
    pub panel_update_download: &'static str,
    pub panel_mark_problem: &'static str,
    pub panel_mark_problem_hint: &'static str,
    pub panel_export_diagnostics: &'static str,

    pub missing_tools_title: &'static str,
    pub missing_tools_body: &'static str,
    pub broken_tools_title: &'static str,
    pub broken_tools_body: &'static str,

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

static TURKISH: Text = Text {
    status_tools_missing: "Araçlar eksik (enjeksiyon devre dışı)",
    status_waiting_league: "League bekleniyor",
    status_connected: "League'e bağlandı",
    status_lobby: "Lobide",
    status_matchmaking: "Maç aranıyor",
    status_ready_check: "Maç bulundu",
    status_champ_select: "Şampiyon seçimi",
    status_finalization: "Seçim tamamlanıyor",
    status_injecting: "Skin uygulanıyor…",
    status_in_game: "Oyunda",
    status_in_game_confirmed: "Oyunda — skin aktif",
    status_in_game_unconfirmed: "Oyunda — skin doğrulanmadı",
    status_in_game_failed: "Oyunda — enjeksiyon başarısız",
    status_reconnecting: "Yeniden bağlanıyor",

    party_off: "Parti: kapalı",
    party_unavailable: "Parti: kullanılamıyor (relay yapılandırılmamış)",
    party_connecting: "Parti: bağlanıyor…",
    party_in_room: "Parti: odada (toplam {n} kişi)",
    party_reconnecting: "Parti: yeniden bağlanıyor…",
    party_created_connecting: "Parti oluşturuldu: bağlanıyor…",
    party_created_in_room: "Parti oluşturuldu: odada (toplam {n} kişi)",

    menu_party_create: "Parti odası oluştur...",
    menu_party_join: "Parti odasına katıl...",
    menu_party_leave: "Partiden ayrıl",
    menu_group_party: "Parti",
    menu_group_folders: "Klasörler",
    menu_open_mods: "Mod klasörünü aç",
    menu_open_logs: "Log klasörünü aç",
    menu_open_tools: "Araçlar klasörünü aç",
    menu_about: "Dekan Hakkında...",
    menu_autostart: "Windows ile başlat",
    menu_auto_accept: "Maçları otomatik kabul et",
    menu_quit: "Dekan'dan çık",
    menu_open_panel: "Dekan'ı aç",
    menu_random_skin: "Seçim yapılmazsa rastgele skin kullan",
    panel_section_options: "Seçenekler",
    panel_section_diagnostics: "Tanılama",
    panel_random_skin_hint: "Şampiyon kilitlendiğinde Dekan'da skin seçilmemişse maçın skinsiz başlamaması için rastgele bir skin seçilir.",
    check_injector: "Enjektör (tools klasörü)",
    check_game: "Yüklü oyun",
    check_client: "League istemcisi",
    check_dll: "Enjektör DLL geçerliliği",
    check_privileges: "Dekan yetkileri",
    detail_ok: "Tamam",
    detail_injector_missing: "ltk_patcher_host.exe veya ltk_patcher_dll.dll eksik",
    detail_game_missing: "oyun klasörü bulunamadı",
    detail_client_connected: "bağlandı",
    detail_client_waiting: "istemcinin açılması bekleniyor",
    detail_dll_days_left: "mevcut yamayı kabul ediyor; {n} gün veya daha sonra oluşturulan oyun derlemelerini reddeder",
    detail_dll_refused: "yüklü yama DLL'in kabul ettiğinden daha yeni: güncel bir DLL çıkana kadar hiçbir skin yüklenmez",
    detail_dll_unknown: "oyun derlemesi okunamadı",
    detail_elevated: "yönetici olarak çalışıyor",
    detail_not_elevated: "yönetici yetkisi olmadan çalışıyor",
    update_available_title: "Dekan {version} kullanılabilir",
    update_available_body: "İndirme sayfasını açmak için buraya tıklayın. Siz onaylamadan hiçbir şey indirilmez veya kurulmaz.",
    panel_update_line: "{version} sürümü kullanılabilir (sizde {current} var).",
    panel_update_download: "İndirme sayfasını aç",
    panel_mark_problem: "Sorunu şimdi işaretle",
    panel_mark_problem_hint: "Oyundan çıkmadan: maç sırasında Ctrl+Shift+B bir sorun gördüğünüz anı işaretler, F12 ekran görüntüsü alır. Maç bittiğinde tanılama verileri otomatik olarak log klasörüne kaydedilir.",
    panel_export_diagnostics: "Tanılama verilerini dışa aktar",

    missing_tools_title: "Dekan — Enjektör Gerekli",
    missing_tools_body: "Dekan'ın çalışması için enjeksiyon altyapısı gerekir:\n• ltk_patcher_host.exe\n• ltk_patcher_dll.dll\n\nİki dosyayı da LTK Manager 1.21.0–1.24.0 sürümlerinden (README'deki 2. adım) 'tools' klasörüne kopyalayın ve Dekan'ı yeniden açın.\nKlasör sizin için açıldı.",
    broken_tools_title: "Dekan — Geçersiz Enjektör",
    broken_tools_body: "'tools' klasöründeki enjektör dosyaları denetlenmiş sürümlerle eşleşmiyor.\n\nBaşlatmadan önce doğru dosyalarla değiştirin.\nKlasör sizin için açıldı.",

    already_running_title: "Dekan zaten açık",
    already_running_body: "Dekan zaten arka planda çalışıyor.\n\nWindows sistem tepsisinde Dekan simgesini bulun.\nKapatmak için simgeye sağ tıklayıp \"Dekan'dan çık\" seçeneğini seçin.",

    party_unavailable_title: "Parti kullanılamıyor",
    party_unavailable_body: "Parti modu için yapılandırılmış bir relay gerekir.\n\n{reason}",
    party_created_title: "Parti odası oluşturuldu",
    party_created_body: "Oda kodu kopyalandı. Arkadaşlarınıza gönderin — kod 1 saat geçerlidir.\n\nKoda sahip olanlar seçtiğiniz skini görebilir.",
    party_copy_failed_body: "Kod kopyalanamadı. Elle kopyalayın:\n\n{code}",
    party_join_title: "Partiye katıl",
    party_join_empty_clipboard: "Arkadaşınızın gönderdiği oda kodunu kopyalayıp tekrar deneyin.",
    party_join_clipboard_error: "Pano okunamadı: {error}",
    party_joining: "Odaya katılınıyor. Durum sistem tepsisi menüsünde gösterilir.",
    party_invalid_code: "Geçersiz parti kodu: {error}",

    party_dialog_create_title: "Parti Odası Oluşturuldu",
    party_dialog_create_desc: "Özel skinlerinizi görebilmeleri için bu kodu aynı takımdaki arkadaşlarınıza gönderin:",
    party_dialog_join_title: "Parti Odasına Katıl",
    party_dialog_join_desc: "Arkadaşınızın gönderdiği oda kodunu girin veya yapıştırın:",
    party_dialog_label_code: "Oda Kodu (Parti)",
    party_dialog_placeholder: "Oda kodunu buraya yapıştırın (DEKAN1:...)",
    party_dialog_btn_copy: "Kodu Kopyala",
    party_dialog_btn_paste: "Yapıştır",
    party_dialog_btn_ok: "Tamam",
    party_dialog_btn_join: "Odaya Katıl",
    party_dialog_btn_cancel: "İptal",
    party_dialog_copied: "Kopyalandı! ✓",
    party_dialog_error_empty: "Lütfen oda kodunu girin.",

    import_title: "Dekan — mod içe aktar",
    import_refused: "Dosya içe aktarılamadı:\n{reason}",
    import_unsupported_extension: "yalnızca .fantome ve .zip modları içe aktarılabilir",
    import_not_a_mod: "geçerli bir mod paketi değil ({error})",
    import_no_manifest: "META/info.json manifest dosyası eksik",
    import_no_content: "pakette WAD/ veya RAW/ içeriği yok",
    import_no_champion: "paket hangi şampiyon için olduğunu belirtmiyor: şampiyon seçiminde şampiyonu seçip yeniden içe aktarın",
    import_io_error: "mod yazılamadı: {error}",

    html_lang: "tr",
    welcome_active: "SİSTEM TEPSİSİNDE AKTİF",
    welcome_background: "Dekan arka planda küçültülmüş olarak çalışmaya devam eder",
    welcome_author: "Proje Isllan Toso tarafından geliştirilmiştir.",
    welcome_tray_hint: "Kontrol panelini açmak için sistem tepsisi simgesine tıklayın: seçenekler, parti, mod ve log klasörleri.",
    welcome_dismiss: "ANLADIM",
    welcome_quote: "",
    party_room_full: "Parti odası dolu. Birinin ayrılmasını isteyip tekrar katılmayı deneyin.",

    about_title: "DEKAN HAKKINDA",
    about_educational: "Tersine mühendislik, oyunun dosya biçimleri ve Windows enjeksiyonu üzerine çalışma amacı taşıyan eğitimsel ve ticari olmayan bir projedir. Kullanım sorumluluğu size aittir: istemciyi değiştirmek Riot Games'in Hizmet Koşulları'nı ihlal eder ve hesabın yasaklanmasına yol açabilir. Dekan, Riot Games ile bağlantılı değildir.",
    about_quote: "“Ben her zaman önce ateş ederim.” — Miss Fortune",
    about_dismiss: "KAPAT",
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
    party_created_connecting: "Party created: connecting…",
    party_created_in_room: "Party created: in the room ({n} in total)",

    menu_party_create: "Create party room...",
    menu_party_join: "Join party room...",
    menu_party_leave: "Leave party",
    menu_group_party: "Party",
    menu_group_folders: "Folders",
    menu_open_mods: "Open mods folder",
    menu_open_logs: "Open logs folder",
    menu_open_tools: "Open tools folder",
    menu_about: "About Dekan...",
    menu_autostart: "Start with Windows",
    menu_auto_accept: "Accept matches automatically",
    menu_quit: "Quit Dekan",
    menu_open_panel: "Open Dekan",
    menu_random_skin: "Random skin if none is chosen",
    panel_section_options: "Options",
    panel_section_diagnostics: "Diagnostics",
    panel_random_skin_hint: "When your champion locks in with no skin chosen in Dekan, one is rolled so the match never starts without a skin.",
    check_injector: "Injector (tools folder)",
    check_game: "Installed game",
    check_client: "League client",
    check_dll: "Injector DLL validity",
    check_privileges: "Dekan privileges",
    detail_ok: "OK",
    detail_injector_missing: "ltk_patcher_host.exe or ltk_patcher_dll.dll is missing",
    detail_game_missing: "game folder not found",
    detail_client_connected: "connected",
    detail_client_waiting: "waiting for the client to open",
    detail_dll_days_left: "accepts the current patch; refuses game builds made {n} day(s) from now or later",
    detail_dll_refused: "the installed patch is newer than the DLL accepts: no skin loads until a refreshed DLL ships",
    detail_dll_unknown: "game build not read",
    detail_elevated: "running as administrator",
    detail_not_elevated: "running without elevation",
    update_available_title: "Dekan {version} is available",
    update_available_body: "Click here to open the download page. Nothing is downloaded or installed without you.",
    panel_update_line: "Version {version} is available (you have {current}).",
    panel_update_download: "Open download page",
    panel_mark_problem: "Mark a problem now",
    panel_mark_problem_hint: "Without leaving the game: during a match, Ctrl+Shift+B marks the moment something looks wrong and F12 takes a screenshot. When the match ends the diagnostics are saved to the logs folder on their own.",
    panel_export_diagnostics: "Export diagnostics",

    missing_tools_title: "Dekan — Injector Required",
    missing_tools_body: "Dekan requires the injection backend to operate:\n• ltk_patcher_host.exe\n• ltk_patcher_dll.dll\n\nCopy both from LTK Manager 1.21.0 through 1.24.0 (step 2 of the README) into the 'tools' folder and open Dekan again.\nThe folder has been opened for you.",
    broken_tools_title: "Dekan — Invalid Injector",
    broken_tools_body: "The injector files in the 'tools' folder do not match the audited versions.\n\nPlease replace them with the correct files before starting.\nThe folder has been opened for you.",

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
    welcome_author: "Project developed by Isllan Toso.",
    welcome_tray_hint: "Click the tray icon to open the control panel: options, party, and the mods and logs folders.",
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
