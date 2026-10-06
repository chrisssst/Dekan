use crate::i18n::Language;

#[test]
#[ignore = "writes the rendered pages for the visual tests into DEKAN_UI_DUMP"]
fn dump_rendered_pages() {
    let Ok(dir) = std::env::var("DEKAN_UI_DUMP") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("dump dir");
    std::fs::write(
        dir.join("overlay.html"),
        crate::overlay_window::overlay_html(),
    )
    .expect("overlay");
    for (tag, language) in [("tr", Language::Turkish), ("en", Language::English)] {
        let text = language.text();
        for (page, html) in [
            ("welcome", crate::welcome::welcome_html(text)),
            ("about", crate::welcome::about_html(text)),
            (
                "party-create",
                crate::party_dialog::created_html(text, "BLT-7Q2M-X9KD"),
            ),
            ("party-join", crate::party_dialog::join_html(text, None)),
        ] {
            std::fs::write(dir.join(format!("{page}-{tag}.html")), html).expect("page");
        }
    }
}
