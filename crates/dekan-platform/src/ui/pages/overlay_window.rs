use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use dekan_core::mods::ModSelectionView;
use dekan_core::overlay::{Catalog, ModsPanel, OverlayCommand, PresetsView, SelectionOrigin};
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tracing::{debug, info, warn};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowPos, ShowWindow,
};

use super::overlay_model as model;
use super::runtime;
use super::views::{self, ChromaGem, LobbyChoice, OverlayLabels, SkinCard, SkinRow};
use crate::client_window::{
    ClientWindowState, WindowRect, client_window_state, overlay_placement, overlay_placement_on,
};
use crate::error::PlatformError;
use crate::i18n::{Language, Text};

pub const OVERLAY_WIDTH: i32 = 360;

pub const OVERLAY_HEIGHT: i32 = 520;

pub const OVERLAY_PADDING: i32 = 16;

pub const OVERLAY_MIN_WIDTH: i32 = 320;

pub const OVERLAY_MIN_HEIGHT: i32 = 380;

static OVERLAY_SIZE: (AtomicI32, AtomicI32) = (
    AtomicI32::new(OVERLAY_WIDTH),
    AtomicI32::new(OVERLAY_HEIGHT),
);

#[must_use]
pub fn overlay_size() -> (i32, i32) {
    (
        OVERLAY_SIZE.0.load(Ordering::Relaxed),
        OVERLAY_SIZE.1.load(Ordering::Relaxed),
    )
}

struct Overlay {
    view: views::OverlayWindow,
    hwnd: isize,
    commands: UnboundedSender<OverlayCommand>,
    catalog: Catalog,
    language: Language,
    search: String,
    mods_tab: bool,
    mods: ModsPanel,
    selected: Option<u32>,
    origin: Option<SelectionOrigin>,
    columns: usize,
    tiles: HashMap<u32, slint::Image>,
    previews: HashMap<u32, slint::Image>,
    preview_for: Option<u32>,
    presets: PresetsView,
    expanded: Option<slint::LogicalSize>,
}

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

fn with_overlay(work: impl FnOnce(&mut Overlay)) {
    OVERLAY.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => {
            if let Some(overlay) = slot.as_mut() {
                work(overlay);
            }
        }
        Err(_) => debug!("Overlay update skipped: the overlay is already being updated"),
    });
}

#[derive(Clone)]
pub struct OverlayController {
    hwnd: isize,
    alive: Arc<AtomicBool>,
}

impl OverlayController {
    #[must_use]
    pub fn window_handle(&self) -> isize {
        self.hwnd
    }

    fn post(&self, work: impl FnOnce(&mut Overlay) + Send + 'static) {
        if !self.alive.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = runtime::run_on_ui(move || with_overlay(work)) {
            warn!(error = %e, "Could not reach the overlay window");
        }
    }

    pub fn show_at(&self, rect: WindowRect) {
        self.post(move |overlay| overlay.show_at(rect));
    }

    pub fn hide(&self) {
        self.post(|overlay| overlay.hide());
    }

    pub fn set_catalog(&self, catalog: Catalog) {
        self.post(move |overlay| overlay.set_catalog(catalog));
    }

    pub fn set_selection(&self, entry_id: Option<u32>, origin: Option<SelectionOrigin>) {
        self.post(move |overlay| {
            overlay.selected = entry_id;
            overlay.origin = entry_id.and(origin);
            overlay.render_selection();
        });
    }

    pub fn set_presets(&self, presets: PresetsView) {
        self.post(move |overlay| {
            overlay.presets = presets;
            overlay.render_presets();
            overlay.render_selection();
        });
    }

    pub fn set_mods(&self, panel: ModsPanel) {
        self.post(move |overlay| {
            overlay.mods = panel;
            overlay.render_mods();
        });
    }

    pub fn set_mod_selection(&self, selection: ModSelectionView) {
        self.post(move |overlay| {
            overlay.mods.selection = selection;
            overlay.render_mods();
        });
    }

    pub fn set_chroma_preview(&self, chroma_id: u32, image: Arc<[u8]>) {
        self.post(move |overlay| overlay.deliver_preview(chroma_id, &image));
    }

    pub fn click(&self, entry_id: u32) {
        self.post(move |overlay| overlay.choose(entry_id));
    }

    pub fn shutdown(&self) {
        if !self.alive.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Err(e) = runtime::run_on_ui(|| {
            OVERLAY.with_borrow_mut(|slot| {
                if let Some(overlay) = slot.take() {
                    if let Err(e) = overlay.view.hide() {
                        debug!(error = %e, "The overlay window was already closed");
                    }
                }
            });
        }) {
            debug!(error = %e, "The overlay window was already gone at shutdown");
        }
    }
}

pub struct OverlayWindow {
    controller: OverlayController,
}

impl OverlayWindow {
    pub fn spawn() -> Result<(Self, UnboundedReceiver<OverlayCommand>), PlatformError> {
        let (command_tx, command_rx) = unbounded_channel::<OverlayCommand>();
        let (ready_tx, ready_rx) = channel::<Result<isize, String>>();
        runtime::run_on_ui(move || {
            let created = create(command_tx);
            let ready = ready_tx.clone();
            match created {
                Ok(view) => runtime::when_created(&view, move |view, hwnd| {
                    finish(view, hwnd);
                    if ready.send(Ok(hwnd)).is_err() {
                        debug!("Nobody waited for the overlay window");
                    }
                }),
                Err(e) => {
                    if ready_tx.send(Err(e)).is_err() {
                        debug!("Nobody waited for the overlay window");
                    }
                }
            }
        })?;
        let hwnd = ready_rx
            .recv()
            .map_err(|_| PlatformError::Window("overlay window thread died at startup".into()))?
            .map_err(PlatformError::Window)?;
        info!("Overlay window created");
        Ok((
            Self {
                controller: OverlayController {
                    hwnd,
                    alive: Arc::new(AtomicBool::new(true)),
                },
            },
            command_rx,
        ))
    }

    #[must_use]
    pub fn controller(&self) -> OverlayController {
        self.controller.clone()
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.controller.shutdown();
    }
}

fn create(commands: UnboundedSender<OverlayCommand>) -> Result<views::OverlayWindow, String> {
    let view = views::OverlayWindow::new().map_err(|e| format!("overlay window: {e}"))?;
    runtime::repaint_on_expose(&view, |v| v.set_expose_flip(!v.get_expose_flip()));
    wire(&view);
    view.show().map_err(|e| format!("overlay window: {e}"))?;
    let mut overlay = Overlay::new(view.clone_strong(), commands);
    overlay.apply_language();
    overlay.render_all();
    OVERLAY.with_borrow_mut(|slot| *slot = Some(overlay));
    Ok(view)
}

fn finish(view: &views::OverlayWindow, hwnd: isize) {
    view.window()
        .with_winit_window(|window: &winit::window::Window| {
            use winit::platform::windows::WindowExtWindows;
            window.set_skip_taskbar(true);
        });
    let target = HWND(hwnd as *mut _);
    unsafe {
        let _ = ShowWindow(target, SW_HIDE); // ignore-ok: returns the previous visibility, not an error
    }
    view.window().on_winit_window_event(|_, event| {
        if let winit::event::WindowEvent::Resized(size) = event {
            let (width, height) = (size.width as i32, size.height as i32);
            if width > 0 && height > 0 {
                OVERLAY_SIZE.0.store(width, Ordering::Relaxed);
                OVERLAY_SIZE.1.store(height, Ordering::Relaxed);
            }
        }
        slint::winit_030::EventResult::Propagate
    });
    with_overlay(|overlay| overlay.hwnd = hwnd);
}

fn wire(view: &views::OverlayWindow) {
    view.on_search_edited(|search| {
        with_overlay(|overlay| {
            overlay.search = search.to_string();
            overlay.render_list();
        });
    });
    view.on_search_focused(|| {
        with_overlay(|overlay| crate::client_window::take_foreground(overlay.hwnd));
    });
    view.on_search_done(crate::client_window::return_foreground_to_client);
    view.on_choose(|id| {
        if let Ok(id) = u32::try_from(id) {
            with_overlay(|overlay| overlay.choose(id));
        }
    });
    view.on_random(|| with_overlay(|overlay| overlay.send(OverlayCommand::Random)));
    view.on_show_tab(|mods_tab| {
        with_overlay(|overlay| {
            if overlay.mods_tab != mods_tab {
                overlay.mods_tab = mods_tab;
                overlay.search.clear();
                overlay.view.set_search(SharedString::default());
                overlay.view.set_mods_tab(mods_tab);
                overlay.render_list();
            }
        });
    });
    view.on_mod_clicked(|slot, id| {
        with_overlay(|overlay| {
            if let Some(next) = model::toggle_mod(&overlay.mods.selection, &slot, &id) {
                overlay.mods.selection = next.clone();
                overlay.render_mods();
                overlay.send(OverlayCommand::SetMods { selection: next });
            }
        });
    });
    view.on_import_mod(|index| {
        let categories = model::import_categories();
        if let Some(category) = usize::try_from(index).ok().and_then(|i| categories.get(i)) {
            let category = *category;
            with_overlay(|overlay| overlay.send(OverlayCommand::ImportMod { category }));
        }
    });
    view.on_open_mods_folder(|| {
        with_overlay(|overlay| overlay.send(OverlayCommand::OpenModsFolder))
    });
    view.on_gem_hovered(|gem, x, y, on| {
        with_overlay(|overlay| {
            if on && gem.has_preview {
                overlay.show_preview(&gem, x, y);
            } else if on {
                debug!(
                    chroma_id = gem.id,
                    "Hovered chroma has no preview image in the catalog"
                );
            } else {
                overlay.hide_preview();
            }
        });
    });
    view.on_pin_clicked(|| with_overlay(|overlay| overlay.send(OverlayCommand::TogglePreset)));
    view.on_profile_chosen(|index| {
        with_overlay(|overlay| {
            let name = usize::try_from(index)
                .ok()
                .and_then(|index| overlay.presets.profiles.get(index).cloned());
            if let Some(name) = name {
                overlay.send(OverlayCommand::SetProfile { name });
            }
        });
    });
    view.on_profile_added(|| with_overlay(|overlay| overlay.send(OverlayCommand::NewProfile)));
    view.on_profile_removed(|| {
        with_overlay(|overlay| overlay.send(OverlayCommand::DeleteProfile));
    });
    view.on_focus_champion(|id| {
        if let Ok(id) = u32::try_from(id) {
            with_overlay(|overlay| overlay.send(OverlayCommand::FocusChampion { id }));
        }
    });
    view.on_scrolled(|| with_overlay(Overlay::hide_preview));
    view.on_columns_changed(|columns| {
        with_overlay(|overlay| {
            overlay.columns = usize::try_from(columns).unwrap_or(1).max(1);
            overlay.render_rows();
        });
    });
    view.on_drag(|| {
        with_overlay(|overlay| {
            overlay.hide_preview();
            overlay
                .view
                .window()
                .with_winit_window(|window: &winit::window::Window| {
                    if let Err(e) = window.drag_window() {
                        debug!(error = %e, "The overlay window could not be dragged");
                    }
                });
        });
    });
    view.on_resize(|| {
        with_overlay(|overlay| {
            overlay.hide_preview();
            overlay
                .view
                .window()
                .with_winit_window(|window: &winit::window::Window| {
                    if let Err(e) =
                        window.drag_resize_window(winit::window::ResizeDirection::SouthEast)
                    {
                        debug!(error = %e, "The overlay window could not be resized");
                    }
                });
        });
    });
    view.on_hide_requested(|| with_overlay(|overlay| overlay.hide()));
    view.on_minimize(|| with_overlay(Overlay::toggle_collapsed));
}

impl Overlay {
    fn new(view: views::OverlayWindow, commands: UnboundedSender<OverlayCommand>) -> Self {
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
            expanded: None,
        }
    }

    fn text(&self) -> &'static Text {
        self.language.text()
    }

    fn send(&self, command: OverlayCommand) {
        if matches!(command, OverlayCommand::ChromaPreview { .. }) {
            debug!(?command, "Overlay UI command received");
        } else {
            info!(?command, "Overlay UI command received");
        }
        if self.commands.send(command).is_err() {
            debug!("Nobody is listening for overlay commands any more");
        }
    }

    fn show_at(&self, rect: WindowRect) {
        let target = HWND(self.hwnd as *mut _);
        let (x, y, width, height) = (rect.left, rect.top, rect.width(), rect.height());
        unsafe {
            let _ = SetWindowPos(target, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE); // ignore-ok: a refused reposition is retried by the next tracking tick, 200 ms later
            let _ = ShowWindow(target, SW_SHOWNOACTIVATE); // ignore-ok: returns the previous visibility, not an error
        }
    }

    fn toggle_collapsed(&mut self) {
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

    fn hide(&mut self) {
        if self.expanded.is_some() {
            self.toggle_collapsed();
        }
        self.hide_preview();
        unsafe {
            let _ = ShowWindow(HWND(self.hwnd as *mut _), SW_HIDE); // ignore-ok: returns the previous visibility, not an error
        }
    }

    fn set_catalog(&mut self, catalog: Catalog) {
        self.language = Language::for_locale(catalog.locale.as_deref());
        self.tiles = catalog
            .skins
            .iter()
            .filter_map(|skin| {
                let image = runtime::image(skin.tile.as_deref()?)?;
                Some((skin.id, image))
            })
            .collect();
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

    fn choose(&mut self, entry_id: u32) {
        let (selected, command) = model::choose(self.selected, entry_id);
        self.selected = selected;
        self.origin = None;
        self.render_selection();
        self.send(command);
    }

    fn apply_language(&self) {
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

    fn render_all(&mut self) {
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
        self.view.set_mods_tab(self.mods_tab);
        self.render_list();
        self.render_selection();
    }

    fn render_list(&mut self) {
        self.hide_preview();
        self.render_rows();
        self.render_mods();
    }

    fn render_rows(&self) {
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

    fn card(&self, skin: &dekan_core::overlay::CatalogSkin) -> SkinCard {
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

    fn render_mods(&self) {
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

    fn render_selection(&self) {
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

    fn render_presets(&self) {
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

    fn show_preview(&mut self, gem: &ChromaGem, x: f32, y: f32) {
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

    fn hide_preview(&mut self) {
        self.preview_for = None;
        self.view.set_preview_visible(false);
    }

    fn deliver_preview(&mut self, chroma_id: u32, bytes: &[u8]) {
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

pub(crate) fn parse_color(color: Option<&str>) -> slint::Color {
    let fallback = slint::Color::from_rgb_u8(0x3a, 0x4a, 0x5a);
    let Some(hex) = color.and_then(|c| c.strip_prefix('#')) else {
        return fallback;
    };
    match (hex.len(), u32::from_str_radix(hex, 16)) {
        (6, Ok(rgb)) => slint::Color::from_argb_encoded(0xff00_0000 | rgb),
        _ => fallback,
    }
}

#[must_use]
pub fn decide_placement(
    state: ClientWindowState,
    wanted: bool,
    monitor: Option<WindowRect>,
) -> Option<WindowRect> {
    if !wanted {
        return None;
    }
    match state {
        ClientWindowState::Visible(rect) => {
            let (width, height) = overlay_size();
            Some(overlay_placement_on(
                rect,
                monitor,
                width,
                height,
                OVERLAY_PADDING,
            ))
        }
        ClientWindowState::Hidden | ClientWindowState::Absent => None,
    }
}

#[derive(Debug, Default)]
pub struct OverlayTracker {
    last_client_rect: Option<WindowRect>,
    was_wanted: bool,
}

impl OverlayTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, controller: &OverlayController, wanted: bool) -> Option<WindowRect> {
        if !wanted {
            if self.was_wanted {
                controller.hide();
                self.was_wanted = false;
                self.last_client_rect = None;
            }
            return None;
        }

        match client_window_state() {
            ClientWindowState::Visible(client_rect) => {
                let (width, height) = overlay_size();
                let rect = overlay_placement(client_rect, width, height, OVERLAY_PADDING);

                if self.last_client_rect != Some(client_rect) {
                    controller.show_at(rect);
                    self.last_client_rect = Some(client_rect);
                }
                self.was_wanted = true;
                Some(rect)
            }
            ClientWindowState::Hidden | ClientWindowState::Absent => {
                if self.was_wanted {
                    controller.hide();
                    self.was_wanted = false;
                    self.last_client_rect = None;
                }
                None
            }
        }
    }
}

static GLOBAL_TRACKER: Mutex<Option<OverlayTracker>> = Mutex::new(None);

pub fn track_once(controller: &OverlayController, wanted: bool) -> Option<WindowRect> {
    let mut lock = GLOBAL_TRACKER.lock().unwrap_or_else(|e| e.into_inner());
    let tracker = lock.get_or_insert_with(OverlayTracker::new);
    tracker.tick(controller, wanted)
}

#[cfg(test)]
#[path = "overlay_window_tests.rs"]
mod tests;
