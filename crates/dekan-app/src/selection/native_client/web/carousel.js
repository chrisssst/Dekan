
  function findParentSkin(entryId) {
    if (!state.catalog || !Array.isArray(state.catalog.skins)) return null;
    for (const skin of state.catalog.skins) {
      if (skin.id === entryId) return skin;
      if (Array.isArray(skin.chromas) && skin.chromas.some((c) => c.id === entryId)) return skin;
    }
    return null;
  }
  function findSelectedVariant(skin) {
    if (!skin || !Array.isArray(skin.chromas)) return null;
    return skin.chromas.find((c) => c.id === state.selectedId) || null;
  }
  function extractSkinIdFromNode(node) {
    if (!node || !state.catalog || !Array.isArray(state.catalog.skins)) return null;
    try {
      const emberEl = (node.classList && node.classList.contains("ember-view")) ? node : (node.closest && node.closest(".ember-view"));
      if (emberEl) {
        const views = window.Ember && window.Ember.View && window.Ember.View.views;
        const view = emberEl.__ember_view__ || emberEl._view || (views && emberEl.id && views[emberEl.id]);
        const ctx = view && (view.context || view._context || (view.get && view.get("context")));
        const skinObj = ctx && (ctx.skin || (ctx.item && ctx.item.skin) || ctx);
        const emberSkinId = skinObj && (skinObj.id || skinObj.skinId);
        if (typeof emberSkinId === "number") return emberSkinId;
      }
    } catch (_) {}
    const thumb = node.querySelector(".skin-selection-thumbnail") || node;
    const inlineBg = thumb.getAttribute("style") || "";
    let compBg = "";
    try { compBg = window.getComputedStyle(thumb).backgroundImage || ""; } catch (_) {}
    const blob = (node.innerHTML || "") + " " + inlineBg + " " + compBg;
    const match = blob.match(/champion-(?:splashes|tiles)\/(\d+)\/(\d+)\.(?:jpg|png)/i) || blob.match(/\/(\d{4,9})\.(?:jpg|png)/i);
    if (match) {
      const num = parseInt(match[match.length - 1], 10);
      if (state.catalog.skins.some((s) => s.id === num) || num === state.catalog.champion_id * 1000) return num;
    }
    const skinDir = blob.match(/\/skins\/skin(\d+)\//i);
    if (skinDir) {
      const num = state.catalog.champion_id * 1000 + parseInt(skinDir[1], 10);
      if (state.catalog.skins.some((s) => s.id === num)) return num;
    }
    if (/\/skins\/base\//i.test(blob)) return state.catalog.champion_id * 1000;
    return null;
  }
  function matchSkinByText(rawText) {
    if (!state.catalog || !Array.isArray(state.catalog.skins)) return null;
    const target = norm(rawText);
    if (!target) return null;
    if (target === norm(state.catalog.champion_name)) return state.catalog.champion_id * 1000;
    for (const skin of state.catalog.skins) { if (norm(skin.name) === target) return skin.id; }
    for (const skin of state.catalog.skins) {
      const sn = norm(skin.name);
      if (sn && (target.includes(sn) || sn.includes(target))) return skin.id;
    }
    return null;
  }
  function detectActiveCarouselSkinId() {
    if (!state.catalog || !state.catalog.champion_id) return null;
    const center = document.querySelector(".skin-selection-carousel .skin-selection-item.skin-carousel-offset-2")
      || document.querySelector(".quick-play-skin-select-component .thumbnail-wrapper.active-skin")
      || document.querySelector(".quick-play-skin-select-component .thumbnail-wrapper.selected")
      || document.querySelector(".quick-play-skin-select-component .thumbnail-wrapper[aria-selected=\"true\"]")
      || document.querySelector(".skins-pane .skins-pane__skin-card--center-tile")
      || document.querySelector(".skins-pane .skin-card.selected");
    if (center) {
      const byUrl = extractSkinIdFromNode(center);
      if (byUrl !== null) return byUrl;
    }
    const nameEl = document.querySelector(".skin-selection-carousel-container .skin-name-text")
      || document.querySelector(".skin-name-text")
      || document.querySelector(".skins-pane__skin-title")
      || document.querySelector(".quick-play-skin-select-component .skin-name");
    return nameEl && nameEl.textContent ? matchSkinByText(nameEl.textContent) : null;
  }
  function queueSelect(id, fromUserClick) {
    if (!id) return;
    const baseId = state.catalog ? state.catalog.champion_id * 1000 : 0;
    if (id === baseId) {
      if (!fromUserClick && Date.now() < state.suppressBaseUntil) return;
      state.suppressBaseUntil = 0;
      state.selectedId = null;
      state.queue.push({ type: "clear" });
      return;
    }
    if (state.selectedId === id) return;
    state.selectedId = id;
    state.suppressBaseUntil = Date.now() + 8000;
    state.queue.push({ type: "select", id: id });
  }
  function closeFlyout() {
    state.flyoutSkinId = null;
    const existing = document.getElementById(FLYOUT_ID);
    if (existing) existing.remove();
  }
  function closePopover() {
    const pop = document.getElementById(POPOVER_ID);
    if (pop) pop.remove();
    const btn = document.getElementById(NAV_BTN_ID);
    if (btn) btn.classList.remove("open");
  }
  function positionFlyout(flyout, anchor) {
    const rect = anchor && anchor.getBoundingClientRect ? anchor.getBoundingClientRect() : null;
    const fw = 305;
    if (!rect || rect.width === 0) {
      flyout.style.left = Math.max(12, Math.round((window.innerWidth - fw) / 2)) + "px";
      flyout.style.bottom = "140px";
      return;
    }
    const left = Math.max(12, Math.min(window.innerWidth - fw - 12, Math.round(rect.left + rect.width / 2 - fw / 2)));
    flyout.style.left = left + "px";
    flyout.style.bottom = Math.max(16, Math.round(window.innerHeight - rect.top + 16)) + "px";
  }
  function openFlyoutForSkin(skin, anchor) {
    closeFlyout();
    closePopover();
    if (!skin || !Array.isArray(skin.chromas) || skin.chromas.length === 0) return;
    state.flyoutSkinId = skin.id;
    let flyout;
    try {
      flyout = document.createElement("lol-uikit-flyout-frame");
      flyout.setAttribute("orientation", "top");
      flyout.setAttribute("animated", "false");
      flyout.setAttribute("show", "true");
    } catch (_) {
      flyout = document.createElement("div");
    }
    flyout.id = FLYOUT_ID;
    let content;
    try { content = document.createElement("lc-flyout-content"); } catch (_) { content = document.createElement("div"); }
    const modal = document.createElement("div");
    modal.className = "champ-select-chroma-modal chroma-view";
    const infoBox = document.createElement("div");
    infoBox.className = "chroma-information";
    const imgWrap = document.createElement("div");
    imgWrap.className = "chroma-information-image";
    const previewImg = document.createElement("img");
    imgWrap.appendChild(previewImg);
    infoBox.appendChild(imgWrap);
    const titleBox = document.createElement("div");
    titleBox.className = "child-skin-name";
    infoBox.appendChild(titleBox);
    function setPreview(entry) {
      titleBox.textContent = (entry && entry.name) || skin.name;
      const cid = state.catalog ? state.catalog.champion_id : 0;
      const stdCid = cid >= 60000 ? cid - 60000 : cid;
      const stdSkin = skin.id >= 60000000 ? skin.id - 60000000 : skin.id;
      const splashUrl = cid ? `/lol-game-data/assets/v1/champion-splashes/${stdCid}/${stdSkin}.jpg` : "";
      previewImg.onerror = () => {
        if (previewImg.dataset.fb !== "1" && entry && cid >= 60000 && entry.id >= 60000000) {
          previewImg.dataset.fb = "1";
          previewImg.src = `/lol-game-data/assets/v1/champion-chroma-images/${stdCid}/${entry.id - 60000000}.png`;
        } else if (previewImg.dataset.fb !== "2" && splashUrl) {
          previewImg.dataset.fb = "2";
          previewImg.src = splashUrl;
        }
      };
      previewImg.dataset.fb = "0";
      if (entry && entry.preview_path) previewImg.src = entry.preview_path;
      else if (cid && entry) previewImg.src = `/lol-game-data/assets/v1/champion-chroma-images/${cid}/${entry.id}.png`;
      else if (splashUrl) previewImg.src = splashUrl;
    }
    const regular = skin.chromas.filter((c) => !c.form);
    const forms = skin.chromas.filter((c) => c.form);
    setPreview(findSelectedVariant(skin) || regular[0] || forms[0]);
    modal.appendChild(infoBox);
    if (regular.length > 0) {
      const swatches = document.createElement("div");
      swatches.className = "chroma-selection";
      const baseSwatch = document.createElement("div");
      baseSwatch.className = "dekan-swatch" + (state.selectedId === skin.id || !findSelectedVariant(skin) ? " active" : "");
      baseSwatch.style.background = "linear-gradient(135deg, #c8aa6e 50%, #785a28 50%)";
      baseSwatch.title = skin.name;
      baseSwatch.addEventListener("mouseenter", () => setPreview(null));
      baseSwatch.addEventListener("click", (ev) => { ev.stopPropagation(); queueSelect(skin.id, true); attachChromaButton(); closeFlyout(); });
      swatches.appendChild(baseSwatch);
      for (const chroma of regular) {
        const sw = document.createElement("div");
        sw.className = "dekan-swatch" + (state.selectedId === chroma.id ? " active" : "");
        sw.style.background = chroma.color || "#c8aa6e";
        sw.title = chroma.name || String(chroma.id);
        sw.addEventListener("mouseenter", () => setPreview(chroma));
        sw.addEventListener("click", (ev) => { ev.stopPropagation(); queueSelect(chroma.id, true); attachChromaButton(); closeFlyout(); });
        swatches.appendChild(sw);
      }
      modal.appendChild(swatches);
    }
    if (forms.length > 0) {
      const formWrap = document.createElement("div");
      formWrap.className = "dekan-flyout-forms";
      for (const form of forms) {
        const pill = document.createElement("button");
        pill.type = "button";
        pill.className = "dekan-form-pill" + (state.selectedId === form.id ? " active" : "");
        const dot = document.createElement("span");
        dot.className = "dekan-form-dot";
        dot.style.background = form.color || "#18d4c4";
        pill.appendChild(dot);
        pill.appendChild(document.createTextNode(form.name || String(form.id)));
        pill.addEventListener("mouseenter", () => setPreview(form));
        pill.addEventListener("click", (ev) => { ev.stopPropagation(); queueSelect(form.id, true); attachChromaButton(); closeFlyout(); });
        formWrap.appendChild(pill);
      }
      modal.appendChild(formWrap);
    }
    content.appendChild(modal);
    flyout.appendChild(content);
    document.body.appendChild(flyout);
    positionFlyout(flyout, anchor);
  }
  function attachChromaButton() {
    const center = document.querySelector(".skin-selection-carousel .skin-selection-item.skin-carousel-offset-2")
      || document.querySelector(".quick-play-skin-select-component .thumbnail-wrapper.active-skin")
      || document.querySelector(".skins-pane .skins-pane__skin-card--center-tile");
    document.querySelectorAll("." + CHROMA_BTN_CLASS).forEach((el) => { if (!center || !center.contains(el)) el.remove(); });
    if (!center || !state.catalog || !state.catalog.champion_id) return;
    const skinId = state.lastCarouselSkinId || (findParentSkin(state.selectedId) && findParentSkin(state.selectedId).id);
    const skin = state.catalog.skins.find((s) => s.id === skinId) || findParentSkin(state.selectedId);
    let btn = center.querySelector("." + CHROMA_BTN_CLASS);
    if (!skin || !Array.isArray(skin.chromas) || skin.chromas.length === 0) { if (btn) btn.remove(); return; }
    if (!btn) {
      btn = document.createElement("div");
      btn.className = CHROMA_BTN_CLASS;
      btn.title = "Chromas / Forms";
      const core = document.createElement("div");
      core.className = "dekan-chroma-core";
      btn.appendChild(core);
      center.appendChild(btn);
    }
    const core = btn.querySelector(".dekan-chroma-core");
    const active = findSelectedVariant(skin);
    const onlyForms = skin.chromas.every((c) => c.form);
    if (core) {
      const wantBg = active
        ? (active.color || (active.form ? "linear-gradient(135deg, #18d4c4, #0a7a82)" : ""))
        : (onlyForms ? "linear-gradient(135deg, #18d4c4, #d2b26e)" : "");
      if (core.dataset.dekanColor !== wantBg) { core.dataset.dekanColor = wantBg; core.style.background = wantBg; }
    }
    btn.onclick = (ev) => {
      ev.stopPropagation(); ev.stopImmediatePropagation(); ev.preventDefault();
      if (state.flyoutSkinId === skin.id) closeFlyout(); else openFlyoutForSkin(skin, btn);
    };
  }
  function pollCarouselSelection() {
    ensureStyle();
    ensureNavButton();
    installWsSuppressionHook();
    syncIdentityAndParty();
    if (!state.catalog || !state.catalog.champion_id) {
      closeFlyout();
      return;
    }
    unlockSwiftplayDom();
    syncSwiftplayBanners();
    syncCarouselOffsets();
    relabelClassicLockedSkin();
    const detected = detectActiveCarouselSkinId();
    if (detected !== null) {
      if (detected === state.lastCarouselSkinId) {
        state.clickLockUntil = 0;
      } else if (Date.now() >= state.clickLockUntil) {
        const baseId = state.catalog.champion_id * 1000;
        if (!(detected === baseId && state.selectedId !== null)) {
          state.lastCarouselSkinId = detected;
          const parentOfSelected = findParentSkin(state.selectedId);
          if (!parentOfSelected || parentOfSelected.id !== detected) queueSelect(detected, false);
        }
      }
    }
    attachChromaButton();
  }
  state.clickHandler = (ev) => {
    handleIdentityClick(ev);
    if (ev.target && ev.target.closest && ev.target.closest("." + CHROMA_BTN_CLASS)) return;
    const flyout = document.getElementById(FLYOUT_ID);
    if (flyout && !flyout.contains(ev.target)) closeFlyout();
    const pop = document.getElementById(POPOVER_ID);
    if (pop && !pop.contains(ev.target) && !ev.target.closest("#" + NAV_BTN_ID)) closePopover();
    const item = ev.target.closest && ev.target.closest(".skin-selection-item, .thumbnail-wrapper, .skin-card, .skins-pane__skin-card");
    if (item && state.catalog && state.catalog.champion_id) {
      const directId = extractSkinIdFromNode(item);
      if (directId !== null) {
        state.lastCarouselSkinId = directId;
        state.clickLockUntil = Date.now() + 1200;
        queueSelect(directId, true);
        attachChromaButton();
      } else {
        schedulePoll();
      }
    }
  };
  state.keyHandler = (ev) => { if (ev.key === "Escape") { closeFlyout(); closePopover(); } };
  document.addEventListener("click", state.clickHandler, true);
  document.addEventListener("keydown", state.keyHandler, true);
  function isOwnNode(node) {
    const el = node && node.nodeType === 1 ? node : node && node.parentElement;
    return Boolean(el && el.closest && el.closest("#" + FLYOUT_ID + ", ." + CHROMA_BTN_CLASS + ", #" + NAV_BTN_ID + ", #" + POPOVER_ID + ", #" + PARTY_BTN_ID + ", .dekan-tier-bar"));
  }
  function isOwnRecord(rec) {
    if (isOwnNode(rec.target)) return true;
    const moved = Array.from(rec.addedNodes).concat(Array.from(rec.removedNodes));
    return rec.type === "childList" && moved.length > 0 && moved.every(isOwnNode);
  }
  function schedulePoll() {
    if (state.pollTimer) return;
    state.pollTimer = defer(() => { state.pollTimer = null; pollCarouselSelection(); }, 150);
  }
  function setObserverActive(active) {
    if (active) {
      if (state.observer) return;
      state.observer = new MutationObserver((records) => { if (!records.every(isOwnRecord)) schedulePoll(); });
      state.observer.observe(document.body || document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
    } else {
      if (state.observer) { state.observer.disconnect(); state.observer = null; }
      if (state.pollTimer) { clearTimeout(state.pollTimer); state.pollTimer = null; }
    }
  }
  window.__dekanInClient = {
    version: 18,
    sync(catalog, selectedId) {
      const prevChamp = state.catalog ? state.catalog.champion_id : null;
      state.catalog = catalog;
      if (prevChamp !== (catalog ? catalog.champion_id : null)) { state.lastCarouselSkinId = null; closeFlyout(); }
      if (selectedId !== undefined && selectedId !== state.selectedId) {
        state.selectedId = selectedId;
        if (selectedId) state.suppressBaseUntil = Date.now() + 8000;
      }
      setObserverActive(Boolean(catalog && catalog.champion_id > 0));
      pollCarouselSelection();
      syncIdentityAndParty();
      if (document.getElementById(POPOVER_ID)) renderNavPopover(document.getElementById(NAV_BTN_ID));
      return state.queue.splice(0, state.queue.length);
    },
    drain(rustSelectedId) {
      if (state.queue.length === 0 && rustSelectedId !== undefined && rustSelectedId !== state.selectedId) {
        state.selectedId = rustSelectedId;
        if (rustSelectedId) state.suppressBaseUntil = Date.now() + 8000;
      }
      pollCarouselSelection();
      syncIdentityAndParty();
      return state.queue.splice(0, state.queue.length);
    },
    cleanup() {
      state.disposed = true;
      state.timeouts.forEach(clearTimeout);
      state.timeouts.clear();
      if (state.observer) { state.observer.disconnect(); state.observer = null; }
      if (state.pollTimer) { clearTimeout(state.pollTimer); state.pollTimer = null; }
      if (state.clickHandler) { document.removeEventListener("click", state.clickHandler, true); state.clickHandler = null; }
      if (state.keyHandler) { document.removeEventListener("keydown", state.keyHandler, true); state.keyHandler = null; }
      if (state.origMsgDataDesc) {
        try { Object.defineProperty(MessageEvent.prototype, "data", state.origMsgDataDesc); } catch (_) {}
        state.origMsgDataDesc = null;
      }
      closeFlyout();
      closePopover();
      cleanupIdentityAndParty();
      const navBtn = document.getElementById(NAV_BTN_ID);
      if (navBtn) navBtn.remove();
      document.querySelectorAll(".dekan-nav-sep").forEach((el) => el.remove());
      const style = document.getElementById(STYLE_ID);
      if (style) style.remove();
      const legacy = document.getElementById("dekan-in-client-panel");
      if (legacy) legacy.remove();
      document.querySelectorAll("." + CHROMA_BTN_CLASS).forEach((el) => el.remove());
      delete window.__dekanInClient;
      return true;
    }
  };
  syncIdentityAndParty();
  return true;
