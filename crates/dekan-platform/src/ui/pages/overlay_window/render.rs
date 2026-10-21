use super::*;

impl Overlay {
    pub(super) fn new(
        view: views::OverlayWindow,
        commands: UnboundedSender<OverlayCommand>,
    ) -> Self {
        Self {
            view,
            hwnd: 0,
            commands,
            catalog: Catalog::default(),
            language: Language::for_locale(None),
            search: String::new(),
            mods_tab: false,
            mods: ModsPanel::default(),
            selected: None,
            origin: None,
            columns: 1,
            tiles: HashMap::new(),
            previews: HashMap::new(),
            preview_for: None,
            presets: PresetsView::default(),
            party: None,
            expanded: None,
        }
    }

    pub(super) fn text(&self) -> &'static Text {
        self.language.text()
    }

    pub(super) fn send(&self, command: OverlayCommand) {
        if matches!(command, OverlayCommand::ChromaPreview { .. }) {
            debug!(?command, "Overlay UI command received");
        } else {
            info!(?command, "Overlay UI command received");
        }
        if self.commands.send(command).is_err() {
            debug!("Nobody is listening for overlay commands any more");
        }
    }

    pub(super) fn show_at(&self, rect: WindowRect) {
        let target = HWND(self.hwnd as *mut _);
        let (x, y, width, height) = (rect.left, rect.top, rect.width(), rect.height());
        unsafe {
            match SetWindowPos(
                target,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE,
            ) {
                Ok(()) => {
                    if REPOSITION_FAILED.swap(false, Ordering::Relaxed) {
                        debug!("Overlay window repositioned after a refusal");
                    }
                }
                Err(e) => {
                    if !REPOSITION_FAILED.swap(true, Ordering::Relaxed) {
                        warn!(error = %e, "Overlay window could not be repositioned; retrying on the next tracking tick");
                    }
                }
            }
            let _ = ShowWindow(target, SW_SHOWNOACTIVATE); // ignore-ok: returns the previous visibility, not an error
        }
    }

    pub(super) fn toggle_collapsed(&mut self) {
        self.hide_preview();
        let window = self.view.window();
        match self.expanded.take() {
            Some(size) => {
                self.view.set_collapsed(false);
                window.set_size(size);
            }
            None => {
                let size = window.size().to_logical(window.scale_factor());
                self.expanded = Some(size);
                self.view.set_collapsed(true);
                window.set_size(slint::LogicalSize::new(
                    size.width,
                    self.view.get_header_height(),
                ));
            }
        }
    }

    pub(super) fn hide(&mut self) {
        if self.expanded.is_some() {
            self.toggle_collapsed();
        }
        self.hide_preview();
        unsafe {
            let _ = ShowWindow(HWND(self.hwnd as *mut _), SW_HIDE); // ignore-ok: returns the previous visibility, not an error
        }
    }

    pub(super) fn set_catalog(&mut self, catalog: Catalog) {
        self.language = Language::for_locale(catalog.locale.as_deref());
        let mut tiles = HashMap::new();
        let mut undecodable = 0usize;
        let mut first_undecodable = None;
        for skin in &catalog.skins {
            let Some(bytes) = skin.tile.as_deref() else {
                continue;
            };
            match runtime::image(bytes) {
                Some(image) => {
                    tiles.insert(skin.id, image);
                }
                None => {
                    undecodable += 1;
                    first_undecodable.get_or_insert(skin.id);
                }
            }
        }
        if let Some(first_skin) = first_undecodable {
            warn!(
                count = undecodable,
                first_skin, "Skin tiles could not be decoded; their cards show no image"
            );
        }
        self.tiles = tiles;
        self.mods = catalog.mods.clone();
        self.catalog = catalog;
        self.selected = None;
        self.origin = None;
        self.previews.clear();
        self.hide_preview();
        self.search.clear();
        self.view.set_search(SharedString::default());
        self.apply_language();
        self.render_all();
    }

    pub(super) fn choose(&mut self, entry_id: u32) {
        let (selected, command) = model::choose(self.selected, entry_id);
        self.selected = selected;
        self.origin = None;
        self.render_selection();
        self.send(command);
    }

    pub(super) fn apply_language(&self) {
        let text = self.text();
        self.view.set_labels(OverlayLabels {
            version: crate::version::display_version().into(),
            search_skin: text.overlay_search_skin.into(),
            search_mod: text.overlay_search_mod.into(),
            dice: text.overlay_dice.into(),
            minimize: text.overlay_minimize.into(),
            restore: text.overlay_restore.into(),
            hide: text.overlay_hide.into(),
            tab_skins: text.overlay_tab_skins.into(),
            tab_mods: text.overlay_tab_mods.into(),
            historic_tag: text.overlay_historic_tag.into(),
            random_tag: text.overlay_random_tag.into(),
            preset_tag: text.overlay_preset_tag.into(),
            pin: text.overlay_pin.into(),
            unpin: text.overlay_unpin.into(),
            profile_new: text.overlay_profile_new.into(),
            profile_delete: text.overlay_profile_delete.into(),
            connected: text.overlay_connected.into(),
            import_mod: text.overlay_import_mod.into(),
            open_folder: text.overlay_open_folder.into(),
        });
        let categories: Vec<SharedString> = model::import_categories()
            .into_iter()
            .map(|category| model::category_label(category, text).into())
            .collect();
        self.view
            .set_import_categories(ModelRc::new(VecModel::from(categories)));
    }

    pub(super) fn render_all(&mut self) {
        let text = self.text();
        self.view
            .set_champion(model::champion_label(&self.catalog, text).into());
        self.view
            .set_portrait_initial(model::initial(&self.catalog.champion_name).into());
        let portrait = self
            .catalog
            .skins
            .iter()
            .find_map(|skin| self.tiles.get(&skin.id));
        self.view.set_has_portrait(portrait.is_some());
        self.view
            .set_portrait(portrait.cloned().unwrap_or_default());
        self.view
            .set_notice(model::notice(&self.catalog, text).into());
        let choices: Vec<LobbyChoice> = model::lobby_choices(&self.catalog)
            .into_iter()
            .map(|(id, name, active)| LobbyChoice {
                id: i32::try_from(id).unwrap_or(-1),
                name: name.into(),
                active,
            })
            .collect();
        self.view
            .set_lobby_champions(ModelRc::new(VecModel::from(choices)));
        let (footer, quote) = model::footer(&self.catalog, false, text);
        self.view.set_footer_right(footer.into());
        self.view.set_footer_quote(quote);
        self.render_party();
        self.view.set_mods_tab(self.mods_tab);
        self.render_list();
        self.render_selection();
    }

    pub(super) fn render_party(&self) {
        let line = self.party.map_or_else(String::new, |party| {
            let text =
                crate::i18n::fill(self.text().overlay_party, "n", &party.members.to_string());
            crate::i18n::fill(&text, "skins", &party.teammate_skins.to_string())
        });
        self.view.set_party_line(line.into());
    }

    pub(super) fn render_list(&mut self) {
        self.hide_preview();
        self.render_rows();
        self.render_mods();
    }

    pub(super) fn render_rows(&self) {
        let cards: Vec<SkinCard> = model::visible_skins(&self.catalog, &self.search)
            .into_iter()
            .map(|skin| self.card(skin))
            .collect();
        let rows: Vec<SkinRow> = model::chunk(&cards, self.columns)
            .into_iter()
            .map(|cards| SkinRow {
                cards: ModelRc::new(VecModel::from(cards)),
            })
            .collect();
        let (big, sub) = model::empty_texts(&self.catalog, &self.search, self.text());
        self.view.set_empty_big(big.into());
        self.view.set_empty_sub(sub.into());
        self.view.set_rows(ModelRc::new(VecModel::from(rows)));
    }

    pub(super) fn card(&self, skin: &dekan_core::overlay::CatalogSkin) -> SkinCard {
        let tile = self.tiles.get(&skin.id);
        let chromas: Vec<ChromaGem> = skin
            .chromas
            .iter()
            .map(|chroma| ChromaGem {
                id: i32::try_from(chroma.id).unwrap_or(-1),
                name: chroma.name.as_str().into(),
                color: parse_color(chroma.color.as_deref()),
                form: chroma.form,
                has_preview: chroma.has_preview,
            })
            .collect();
        SkinCard {
            id: i32::try_from(skin.id).unwrap_or(-1),
            name: skin.name.as_str().into(),
            name_unknown: skin.name_unknown,
            tile: tile.cloned().unwrap_or_default(),
            has_tile: tile.is_some(),
            initial: model::initial(&skin.name).into(),
            chromas: ModelRc::new(VecModel::from(chromas)),
        }
    }

    pub(super) fn render_mods(&self) {
        let text = self.text();
        let lines = model::mod_lines(
            &self.mods.available,
            &self.mods.selection,
            &self.search,
            text,
        );
        self.view.set_mod_lines(ModelRc::new(VecModel::from(lines)));
        let count = model::selected_count(&self.mods.selection);
        self.view.set_mods_count(if count == 0 {
            SharedString::default()
        } else {
            count.to_string().into()
        });
    }

    pub(super) fn render_selection(&self) {
        let to_int = |id: Option<u32>| id.and_then(|id| i32::try_from(id).ok()).unwrap_or(-1);
        self.view.set_can_pin(self.selected.is_some());
        self.view
            .set_pinned(self.selected.is_some() && self.presets.preset_entry == self.selected);
        self.view.set_selected_id(to_int(self.selected));
        self.view.set_selected_skin(to_int(
            self.selected
                .and_then(|id| model::parent_skin(&self.catalog, id)),
        ));
        self.view.set_origin(match self.origin {
            Some(SelectionOrigin::Historic) => views::SelectionOrigin::Historic,
            Some(SelectionOrigin::Random) => views::SelectionOrigin::Random,
            Some(SelectionOrigin::Preset) => views::SelectionOrigin::Preset,
            None => views::SelectionOrigin::None,
        });
    }

    pub(super) fn render_presets(&self) {
        let text = self.text();
        let names: Vec<SharedString> = self
            .presets
            .profiles
            .iter()
            .map(|name| model::profile_label(name, text).into())
            .collect();
        self.view.set_profiles(ModelRc::new(VecModel::from(names)));
        self.view
            .set_profile_index(i32::try_from(self.presets.active).unwrap_or(0));
        self.view.set_can_delete_profile(self.presets.active > 0);
    }

    pub(super) fn show_preview(&mut self, gem: &ChromaGem, x: f32, y: f32) {
        let Ok(id) = u32::try_from(gem.id) else {
            return;
        };
        self.preview_for = Some(id);
        self.view.set_preview_name(gem.name.clone());
        self.view.set_preview_anchor_x(x);
        self.view.set_preview_anchor_y(y);
        match self.previews.get(&id) {
            Some(image) => {
                self.view.set_preview_image(image.clone());
                self.view.set_preview_loading(false);
            }
            None => {
                self.view.set_preview_image(slint::Image::default());
                self.view.set_preview_loading(true);
                self.send(OverlayCommand::ChromaPreview { id });
            }
        }
        self.view.set_preview_visible(true);
    }

    pub(super) fn hide_preview(&mut self) {
        self.preview_for = None;
        self.view.set_preview_visible(false);
    }

    pub(super) fn deliver_preview(&mut self, chroma_id: u32, bytes: &[u8]) {
        let Some(image) = runtime::image(bytes) else {
            warn!(
                chroma_id,
                bytes = bytes.len(),
                magic = ?bytes.get(..8),
                "A chroma preview could not be decoded; its hover shows no image"
            );
            return;
        };
        let size = image.size();
        debug!(
            chroma_id,
            width = size.width,
            height = size.height,
            shown_now = self.preview_for == Some(chroma_id),
            "Chroma preview decoded"
        );
        if self.preview_for == Some(chroma_id) {
            self.view.set_preview_image(image.clone());
            self.view.set_preview_loading(false);
        }
        self.previews.insert(chroma_id, image);
    }
}
