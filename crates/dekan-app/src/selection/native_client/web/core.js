
  if (window.__dekanInClient && window.__dekanInClient.version === 18) {
    return true;
  }
  if (window.__dekanInClient && typeof window.__dekanInClient.cleanup === "function") {
    try { window.__dekanInClient.cleanup(); } catch (_) {}
  }

  const STYLE_ID = "dekan-in-client-style";
  const CHROMA_BTN_CLASS = "dekan-chroma-btn";
  const FLYOUT_ID = "dekan-chroma-flyout";
  const NAV_BTN_ID = "dekan-nav-btn";
  const POPOVER_ID = "dekan-nav-popover";

  function norm(s) {
    return String(s || "")
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, " ")
      .trim();
  }

  const state = {
    version: 13,
    disposed: false,
    timeouts: new Set(),
    catalog: null,
    selectedId: null,
    lastCarouselSkinId: null,
    suppressBaseUntil: 0,
    clickLockUntil: 0,
    flyoutSkinId: null,
    popoverTab: "mods",
    identity: null,
    lastLobbyFetch: 0,
    lobbyMemberNames: null,
    swiftplayBanners: new Map(),
    queue: [],
    observer: null,
    pollTimer: null,
    clickHandler: null,
    keyHandler: null,
    origMsgDataDesc: null,
  };

  function ensureStyle() {
    if (!document.getElementById(STYLE_ID)) {
      const style = document.createElement("style");
      style.id = STYLE_ID;
      style.textContent = CSS;
      document.head.appendChild(style);
    }
    const legacy = document.getElementById("dekan-in-client-panel");
    if (legacy) legacy.remove();
  }

  function installWsSuppressionHook() {
    if (state.origMsgDataDesc) return;
    try {
      const desc = Object.getOwnPropertyDescriptor(MessageEvent.prototype, "data");
      if (!desc || !desc.configurable || typeof desc.get !== "function") return;
      const origGet = desc.get;
      state.origMsgDataDesc = desc;
      Object.defineProperty(MessageEvent.prototype, "data", {
        configurable: true,
        enumerable: desc.enumerable,
        get() {
          const raw = origGet.call(this);
          if (
            state.selectedId &&
            Date.now() < state.suppressBaseUntil &&
            typeof raw === "string" &&
            raw.includes("/lol-champ-select/v1/skin-selector-info") &&
            (this.target instanceof WebSocket || raw.startsWith("[8,"))
          ) {
            try {
              const parsed = JSON.parse(raw);
              const payload = Array.isArray(parsed) && parsed[2] && parsed[2].data;
              const baseId = state.catalog ? state.catalog.champion_id * 1000 : 0;
              if (
                payload &&
                payload.selectedSkinId !== 0 &&
                (payload.selectedSkinId === baseId || payload.selectedChampionSkinId === baseId)
              ) {
                return '[8,"OnJsonApiEvent",{"uri":"/dekan/suppressed","eventType":"Update","data":null}]';
              }
            } catch (_) {}
          }
          return raw;
        },
      });
    } catch (_) {}
  }

  function unlockSwiftplayDom() {
    document.querySelectorAll(".thumbnail-wrapper.unowned").forEach((el) => {
      el.classList.remove("unowned");
      el.classList.add("owned");
    });
    document.querySelectorAll(".purchase-available").forEach((el) => {
      el.classList.remove("purchase-available");
      el.classList.add("active");
    });
    document.querySelectorAll(".purchase-disabled").forEach((el) => {
      el.classList.remove("purchase-disabled");
    });
  }

  function syncSwiftplayBanners() {
    const active = document.querySelector(".quick-play-skin-select-component .thumbnail-wrapper.active-skin");
    const tile = document.querySelector(".quick-play-loadout-selection-hitbox.selected .champion-slot-tile");
    if (active && tile) {
      const src = tile.getAttribute("src") || "";
      const original = tile.dataset.dekanBanner && src === tile.dataset.dekanBanner ? tile.dataset.dekanOriginal : src;
      const thumb = active.querySelector(".skin-thumbnail-img");
      const m = thumb && /url\(["']?([^"')]+)["']?\)/.exec(thumb.style.backgroundImage || "");
      const picked = m ? m[1].replace("_splash_tile_", "_splash_centered_") : null;
      const folder = (u) => { const r = /\/Characters\/([^/]+)\//i.exec(u || ""); return r ? r[1].toLowerCase() : null; };
      if (original && picked && folder(original) === folder(picked)) {
        if (picked === original) state.swiftplayBanners.delete(original);
        else state.swiftplayBanners.set(original, picked);
      }
    }
    document.querySelectorAll('img[src*="_splash_centered_"]').forEach((img) => {
      const src = img.getAttribute("src");
      if (src !== img.dataset.dekanBanner) img.dataset.dekanOriginal = src;
      const picked = state.swiftplayBanners.get(img.dataset.dekanOriginal);
      if (picked && src !== picked) {
        img.dataset.dekanBanner = picked;
        img.setAttribute("src", picked);
      } else if (!picked && img.dataset.dekanBanner && src === img.dataset.dekanBanner && img.dataset.dekanOriginal) {
        img.setAttribute("src", img.dataset.dekanOriginal);
      }
    });
  }

  function syncCarouselOffsets() {
    document.querySelectorAll(".skin-selection-carousel .skin-selection-item").forEach((el) => {
      const m = el.className.match(/skin-carousel-offset-(-?\d+)/);
      if (!m) return;
      const off = parseInt(m[1], 10);
      const hidden = off < 0 || off > 4;
      if (el.classList.contains("dekan-skin-hidden") !== hidden) {
        el.classList.toggle("dekan-skin-hidden", hidden);
      }
    });
  }

  function relabelClassicLockedSkin() {
    const subtitle = document.querySelector(".skins-pane .skins-pane__sub-title");
    if (!subtitle) return;
    if (!document.querySelector(".skins-pane .skins-pane__skin-card--center-tile .skins-pane__locked-overlay")) return;
    const label = translate("Enabled");
    if (subtitle.textContent.trim() !== label) subtitle.textContent = label;
  }

  function defer(callback, delay) {
    if (state.disposed) return null;
    const id = setTimeout(() => {
      state.timeouts.delete(id);
      if (!state.disposed) callback();
    }, delay);
    state.timeouts.add(id);
    return id;
  }
