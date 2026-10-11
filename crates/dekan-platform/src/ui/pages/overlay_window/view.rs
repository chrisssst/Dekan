use super::*;

pub(super) fn create(
    commands: UnboundedSender<OverlayCommand>,
) -> Result<views::OverlayWindow, String> {
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

pub(super) fn finish(view: &views::OverlayWindow, hwnd: isize) {
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

pub(super) fn wire(view: &views::OverlayWindow) {
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
