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

static REPOSITION_FAILED: AtomicBool = AtomicBool::new(false);

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
    party: Option<dekan_core::party::PartySummary>,
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
        Err(_) => warn!("Overlay update dropped: the overlay is already being updated; the window may show stale data"),
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

    pub fn set_party(&self, party: Option<dekan_core::party::PartySummary>) {
        self.post(move |overlay| {
            overlay.party = party;
            overlay.render_party();
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

mod placement;
mod render;
mod view;

pub use placement::{OverlayTracker, decide_placement, track_once};
use view::{create, finish};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod controls_tests;
