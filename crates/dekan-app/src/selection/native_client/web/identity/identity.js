
  const ID_STYLE_ID = "dekan-id-style";
  const PARTY_BTN_ID = "dekan-party-join-btn";
  const BORDERS_BAR_ID = "dekan-borders-bar";
  const BANNERS_BAR_ID = "dekan-banners-bar";
  const ID_STORAGE_KEY = "dekan_local_identity_v1";
  const BG_STORAGE_KEY = "dekan_local_backdrop_v1";
  const RANKED_TIERS = ["CHALLENGER","GRANDMASTER","MASTER","DIAMOND","EMERALD","PLATINUM","GOLD","SILVER","BRONZE","IRON"];
  const APEX_TIERS = ["CHALLENGER","GRANDMASTER","MASTER"];
  const PRESTIGE_LEVELS = [1,30,50,75,100,125,150,175,200,225,250,275,300,325,350,375,400,425,450,475,500];
  const ID_CSS = `
    .challenges-identity-customizer-root-component .unowned, .challenges-identity-customizer-root-component .unselectable,
    .challenges-identity-customizer-root-component .locked, .challenges-identity-customizer-root-component .disabled,
    .challenges-identity-customizer-root-component .is-locked,
    .identity-customizer-tab-banners-component .identity-customizer-banner.unselectable .identity-customizer-banner-wrapper,
    .challenges-identity-customizer-tab-titles-component .identity-customizer-title-component.unowned,
    .challenges-identity-customizer-tab-titles-component .identity-customizer-title-component.unselectable,
    .challenges-identity-customizer-tab-tokens-component .identity-customizer-token-component.unowned,
    .tft-loadout-item.unowned, .tft-loadout-item.locked, .loadouts-modal .unowned, .loadouts-modal .locked {
      opacity: 1 !important; filter: none !important; pointer-events: auto !important; cursor: pointer !important;
    }
    .identity-customizer-tab-banners-component .identity-customizer-banner-locked,
    .identity-customizer-tab-banners-component .identity-customizer-banner-locked-icon,
    .identity-customizer-tab-banners-component .identity-customizer-banner-locked-background,
    .challenges-identity-customizer-root-component .lock-icon, .challenges-identity-customizer-root-component .locked-overlay,
    .tft-loadout-item .lock-icon, .loadouts-modal .lock-icon { display: none !important; }
    .dekan-tier-bar { display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 10px 10px; margin: 0 0 10px; border: 1px solid #463714; border-radius: 3px; background: rgba(10, 20, 40, 0.85); }
    .dekan-tier-bar-title { width: 100%; font-family: "Beaufort for LOL", sans-serif; font-size: 11px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: #c8aa6e; margin-bottom: 2px; }
    .dekan-tier-chip { display: inline-flex; align-items: center; gap: 5px; padding: 4px 8px; border: 1px solid #463714; border-radius: 2px; background: #0f1a24; color: #cdbe91; font-family: "Spiegel", sans-serif; font-size: 11px; font-weight: 600; cursor: pointer; }
    .dekan-tier-chip:hover { border-color: #c8aa6e; color: #f0e6d2; background: #1e282d; }
    .dekan-tier-chip.active { border-color: #0ac8b9; color: #f0e6d2; background: rgba(10, 200, 185, 0.16); }
    .dekan-tier-chip img { width: 18px; height: 22px; object-fit: cover; border-radius: 1px; }
    #dekan-party-join-btn { display: none; align-items: center; gap: 5px; height: 24px; padding: 0 9px; margin: 0 6px; border: 1px solid #c8aa6e; border-radius: 3px; background: linear-gradient(180deg, #1e282d 0%, #0a1428 100%); color: #f0e6d2; font-family: "Beaufort for LOL", sans-serif; font-size: 11px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; cursor: pointer; -webkit-app-region: no-drag; }
    #dekan-party-join-btn:hover { border-color: #f0e6d2; background: #1e282d; }
    #dekan-party-join-btn .dekan-party-dot { width: 7px; height: 7px; border-radius: 50%; background: #0ac8b9; box-shadow: 0 0 6px #0ac8b9; }
    .dekan-party-badge { display: inline-block; padding: 1px 5px; margin-left: 4px; border-radius: 2px; background: rgba(10,200,185,0.18); border: 1px solid rgba(10,200,185,0.5); color: #0ac8b9; font-size: 9px; font-weight: 700; letter-spacing: .05em; }
  `;

  let cachedIconIds = null;
  let cachedTitleIds = null;
  let cachedSkinPaths = {};
  let catalogsLoading = false;
  let lastChampsLen = 0;
  let lastChampsPatched = null;
  let localSummonerId = null;
  let localPuuid = null;
  let backdropLoaded = false;

  function ensureIdentityStyle() {
    if (!document.getElementById(ID_STYLE_ID)) {
      const st = document.createElement("style");
      st.id = ID_STYLE_ID;
      st.textContent = ID_CSS;
      (document.head || document.documentElement).appendChild(st);
    }
  }

  function getActiveIdentity() {
    if (state.identity) return state.identity;
    const fromParty = state.catalog && state.catalog.party && state.catalog.party.local_identity;
    if (fromParty) {
      state.identity = fromParty;
      try { window.localStorage.setItem(ID_STORAGE_KEY, JSON.stringify(fromParty)); } catch (_) {}
      return fromParty;
    }
    try {
      const raw = window.localStorage && window.localStorage.getItem(ID_STORAGE_KEY);
      if (raw) {
        const parsed = JSON.parse(raw);
        if (parsed && typeof parsed === "object") { state.identity = parsed; return parsed; }
      }
    } catch (_) {}
    return null;
  }

  function formatTierLabel(tier) {
    if (!tier) return "";
    const t = String(tier).toUpperCase();
    const pretty = t.charAt(0) + t.slice(1).toLowerCase();
    return APEX_TIERS.includes(t) ? pretty : (pretty + " I");
  }

  function getStoredBackdrop() {
    if (backdropLoaded) return state.backdrop || null;
    backdropLoaded = true;
    try {
      const raw = window.localStorage && window.localStorage.getItem(BG_STORAGE_KEY);
      state.backdrop = raw ? JSON.parse(raw) : null;
    } catch (_) { state.backdrop = null; }
    return state.backdrop || null;
  }

  function setStoredBackdrop(obj) {
    backdropLoaded = true;
    state.backdrop = obj || null;
    try {
      if (obj) window.localStorage.setItem(BG_STORAGE_KEY, JSON.stringify(obj));
      else window.localStorage.removeItem(BG_STORAGE_KEY);
    } catch (_) {}
  }

  function ensureCatalogsLoaded() {
    if (state.disposed) return;
    if (catalogsLoading) return;
    catalogsLoading = true;
    const rawFetch = state.origFetch || window.fetch;
    if (typeof rawFetch !== "function") return;
    try {
      rawFetch("/lol-game-data/assets/v1/summoner-icons.json").then((r) => r.json()).then((d) => {
        if (Array.isArray(d)) cachedIconIds = d.filter((p) => p && p.id != null && p.id !== 29).map((p) => p.id);
      }).catch(() => {});
      rawFetch("/lol-challenges/v2/titles/all").then((r) => r.json()).then((d) => {
        const arr = Array.isArray(d) ? d : (d && typeof d === "object" ? Object.values(d) : []);
        cachedTitleIds = arr.filter((t) => t && t.contentId).map((t) => ({ contentId: t.contentId, itemId: t.itemId }));
      }).catch(() => {});
    } catch (_) {}
  }

  function recordBackdropSelection(skinId) {
    const n = Number(skinId);
    if (!Number.isFinite(n) || n <= 0) { setStoredBackdrop(null); return; }
    const img = cachedSkinPaths[n] || "";
    if (img) {
      setStoredBackdrop({ skinId: n, backdropImage: img });
      defer(applyLocalIdentityDom, 60);
      return;
    }
    const rawFetch = state.origFetch || window.fetch;
    if (typeof rawFetch === "function") {
      rawFetch("/lol-game-data/assets/v1/skins.json").then((r) => r.json()).then((d) => {
        if (d && typeof d === "object") {
          Object.values(d).forEach((s) => { if (s && s.id && s.splashPath) cachedSkinPaths[s.id] = s.splashPath; });
          if (cachedSkinPaths[n]) {
            setStoredBackdrop({ skinId: n, backdropImage: cachedSkinPaths[n] });
            applyLocalIdentityDom();
          }
        }
      }).catch(() => {});
    }
  }

  function saveIdentityPatch(patch) {
    const base = getActiveIdentity() || {};
    state.identity = Object.assign({}, base, patch);
    if (!state.identity.tokens) state.identity.tokens = [];
    try { window.localStorage.setItem(ID_STORAGE_KEY, JSON.stringify(state.identity)); } catch (_) {}
    state.queue.push({ type: "set_identity", loadout: state.identity });
    applyLocalIdentityDom();
    renderDekanTierBars();
  }

  function isIdentityUrl(url) {
    return Boolean(url && (
      url.includes("/lol-regalia/v3/inventory/REGALIA_BANNER")
      || url.includes("/lol-regalia/v2/current-summoner/regalia")
      || url.includes("/lol-ranked/v1/current-ranked-stats")
      || (localPuuid && url.includes("/lol-ranked/v1/ranked-stats/" + localPuuid))
      || (localSummonerId && url.includes("/lol-collections/v1/inventories/" + localSummonerId + "/backdrop"))
      || (url.includes("/lol-champions/v1/inventories/") && url.endsWith("/champions"))
      || url.includes("/lol-challenges/v1/challenges/local-player")
      || url.includes("/lol-challenges/v2/titles/local-player")
      || url.includes("/lol-challenges/v1/titles/local-player")
      || url.includes("/lol-inventory/v2/inventory/SUMMONER_ICON")
    ));
  }

  function unlockSkinsList(data) {
    data.forEach((c) => {
      if (!c || !Array.isArray(c.skins)) return;
      c.skins.forEach((s) => {
        if (!s) return;
        if (s.id && s.splashPath) cachedSkinPaths[s.id] = s.splashPath;
        if (s.ownership && !s.ownership.owned) s.ownership.owned = true;
        if (s.questSkinInfo && Array.isArray(s.questSkinInfo.tiers)) {
          s.questSkinInfo.tiers.forEach((t) => {
            if (t && t.id && t.splashPath) cachedSkinPaths[t.id] = t.splashPath;
            if (t && t.ownership && !t.ownership.owned) t.ownership.owned = true;
          });
        }
      });
    });
    return data;
  }

  function patchRegaliaJson(url, data) {
    if (state.disposed) return data;
    if (!data || typeof data !== "object") return data;
    const id = getActiveIdentity();
    if (url.includes("/lol-regalia/v3/inventory/REGALIA_BANNER")) {
      Object.keys(data).forEach((k) => { if (data[k] && typeof data[k] === "object") data[k].isOwned = true; });
      return data;
    }
    if (url.includes("/lol-regalia/v2/current-summoner/regalia")) {
      if (document.querySelector(".challenges-identity-customizer-root-component")) data.summonerLevel = 500;
      if (id && id.border && id.border.startsWith("ranked:")) {
        const tier = id.border.slice(7);
        const div = APEX_TIERS.includes(tier) ? "NA" : "I";
        data.crestType = "ranked";
        data.preferredCrestType = "ranked";
        data.highestRankedEntry = Object.assign({}, data.highestRankedEntry || {}, { tier, division: div, queueType: "RANKED_SOLO_5x5" });
      }
      if (id && id.banner) {
        const m = id.banner.match(/BannerSkins\/([a-z]+)\.png$/i);
        const tier = m ? m[1].toUpperCase() : "";
        if (RANKED_TIERS.includes(tier)) {
          data.bannerType = "lastSeasonHighestRank";
          data.preferredBannerType = "lastSeasonHighestRank";
          data.lastSeasonHighestRank = tier;
        }
      }
      return data;
    }
    if ((url.includes("/lol-ranked/v1/current-ranked-stats") || (localPuuid && url.includes("/lol-ranked/v1/ranked-stats/" + localPuuid))) && id && id.border && id.border.startsWith("ranked:")) {
      const tier = id.border.slice(7);
      const div = APEX_TIERS.includes(tier) ? "NA" : "I";
      if (data.highestRankedEntry) Object.assign(data.highestRankedEntry, { tier, division: div, highestTier: tier, highestDivision: div });
      if (Array.isArray(data.queues)) {
        data.queues.forEach((q) => { if (q && q.queueType === "RANKED_SOLO_5x5") Object.assign(q, { tier, division: div, highestTier: tier, highestDivision: div }); });
      }
      return data;
    }
    if (url.includes("/lol-champions/v1/inventories/") && url.endsWith("/champions") && Array.isArray(data) && !(state.catalog && state.catalog.champion_id > 0)) {
      return unlockSkinsList(data);
    }
    if (localSummonerId && url.includes("/lol-collections/v1/inventories/" + localSummonerId + "/backdrop")) {
      const bg = getStoredBackdrop();
      if (bg && bg.backdropImage) { data.backdropType = "specified-skin"; data.backdropImage = bg.backdropImage; data.backdropVideo = ""; }
      return data;
    }
    if (url.includes("/lol-challenges/v1/challenges/local-player") && document.querySelector(".challenges-identity-customizer-root-component")) {
      Object.values(data).forEach((c) => { if (c && c.currentLevel === "NONE") c.currentLevel = "CHALLENGER"; });
      return data;
    }
    if ((url.includes("/lol-challenges/v2/titles/local-player") || url.includes("/lol-challenges/v1/titles/local-player")) && Array.isArray(cachedTitleIds)) {
      const owned = new Set((Array.isArray(data) ? data : []).map((t) => t && t.contentId));
      const out = Array.isArray(data) ? data.slice() : [];
      cachedTitleIds.forEach((t) => { if (!owned.has(t.contentId)) out.push(t); });
      return out;
    }
    if (url.includes("/lol-inventory/v2/inventory/SUMMONER_ICON") && Array.isArray(cachedIconIds)) {
      const owned = new Set((Array.isArray(data) ? data : []).map((i) => i && i.itemId));
      const out = Array.isArray(data) ? data.slice() : [];
      cachedIconIds.forEach((idVal) => { if (!owned.has(idVal)) out.push({ itemId: idVal, purchaseDate: "" }); });
      return out;
    }
    return data;
  }

  function installIdentityNetworkHook() {
    if (state.idNetHooked) return;
    state.idNetHooked = true;
    const origFetch = window.fetch;
    if (typeof origFetch === "function") {
      state.origFetch = origFetch;
      window.fetch = async function (input, init) {
        const res = await origFetch.apply(this, arguments);
        const method = ((init && init.method) || (input && input.method) || "GET").toUpperCase();
        const url = typeof input === "string" ? input : (input && input.url ? input.url : "");
        if (method === "POST" && url.includes("/lol-summoner/v1/current-summoner/summoner-profile") && init && typeof init.body === "string") {
          try { const b = JSON.parse(init.body); if (b && b.key === "backgroundSkinId") recordBackdropSelection(b.value); } catch (_) {}
        }
        if (method === "GET" && res && res.ok && isIdentityUrl(url)) {
          try {
            const patched = patchRegaliaJson(url, await res.clone().json());
            return new Response(JSON.stringify(patched), { status: res.status, statusText: res.statusText, headers: res.headers });
          } catch (_) {}
        }
        return res;
      };
    }
    const XHR = window.XMLHttpRequest;
    if (XHR && XHR.prototype && !state.origXhrOpen) {
      state.origXhrOpen = XHR.prototype.open;
      state.origXhrSend = XHR.prototype.send;
      state.origXhrTextDesc = Object.getOwnPropertyDescriptor(XHR.prototype, "responseText");
      state.origXhrRespDesc = Object.getOwnPropertyDescriptor(XHR.prototype, "response");
      XHR.prototype.open = function (method, url) {
        this.__dekanMethod = String(method || "GET").toUpperCase();
        this.__dekanUrl = String(url || "");
        this.__dekanPatchedText = null;
        return state.origXhrOpen.apply(this, arguments);
      };
      XHR.prototype.send = function (body) {
        if (this.__dekanMethod === "POST" && this.__dekanUrl.includes("/lol-summoner/v1/current-summoner/summoner-profile") && typeof body === "string") {
          try { const b = JSON.parse(body); if (b && b.key === "backgroundSkinId") recordBackdropSelection(b.value); } catch (_) {}
        }
        return state.origXhrSend.apply(this, arguments);
      };
      const getPatchedXhr = (xhr, rawText) => {
        if (xhr.__dekanPatchedText !== null) return xhr.__dekanPatchedText;
        if (xhr.readyState !== 4 || xhr.status !== 200 || xhr.__dekanMethod !== "GET" || !isIdentityUrl(xhr.__dekanUrl)) return rawText;
        const isChamps = xhr.__dekanUrl.includes("/lol-champions/v1/inventories/") && xhr.__dekanUrl.endsWith("/champions") && !(state.catalog && state.catalog.champion_id > 0);
        if (isChamps && lastChampsPatched && rawText.length === lastChampsLen) {
          xhr.__dekanPatchedText = lastChampsPatched;
          return lastChampsPatched;
        }
        try {
          const out = JSON.stringify(patchRegaliaJson(xhr.__dekanUrl, JSON.parse(rawText)));
          xhr.__dekanPatchedText = out;
          if (isChamps) { lastChampsLen = rawText.length; lastChampsPatched = out; }
          return out;
        } catch (_) { return rawText; }
      };
      if (state.origXhrTextDesc && state.origXhrTextDesc.get) {
        Object.defineProperty(XHR.prototype, "responseText", {
          configurable: true,
          get() { return getPatchedXhr(this, state.origXhrTextDesc.get.call(this)); }
        });
      }
      if (state.origXhrRespDesc && state.origXhrRespDesc.get) {
        Object.defineProperty(XHR.prototype, "response", {
          configurable: true,
          get() {
            const raw = state.origXhrRespDesc.get.call(this);
            if (typeof raw === "string") return getPatchedXhr(this, raw);
            if (raw && typeof raw === "object" && this.readyState === 4 && this.status === 200 && this.__dekanMethod === "GET" && isIdentityUrl(this.__dekanUrl)) {
              return patchRegaliaJson(this.__dekanUrl, raw);
            }
            return raw;
          }
        });
      }
    }
    try {
      origFetch("/lol-summoner/v1/current-summoner").then((r) => r.json()).then((s) => {
        if (s) { localSummonerId = s.summonerId || null; localPuuid = s.puuid || null; }
      }).catch(() => {});
      if (typeof window.requestIdleCallback === "function") {
        window.requestIdleCallback(ensureCatalogsLoaded, { timeout: 2500 });
      } else {
        defer(ensureCatalogsLoaded, 1500);
      }
    } catch (_) {}
  }

  function makeTierChip(label, active, imgSrc, onClick) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "dekan-tier-chip" + (active ? " active" : "");
    if (imgSrc) { const img = document.createElement("img"); img.src = imgSrc; btn.appendChild(img); }
    const span = document.createElement("span");
    span.textContent = label;
    btn.appendChild(span);
    btn.onclick = (ev) => { ev.stopPropagation(); ev.preventDefault(); onClick(); };
    return btn;
  }

  function renderDekanTierBars() {
    const id = getActiveIdentity() || {};
    const borderKey = String(id.border || "");
    const bannerKey = String(id.banner || "");
    const bordersTab = document.querySelector(".identity-customizer-tab-borders-component");
    if (bordersTab) {
      let bar = document.getElementById(BORDERS_BAR_ID);
      if (!bar) { bar = document.createElement("div"); bar.id = BORDERS_BAR_ID; bar.className = "dekan-tier-bar"; bordersTab.insertBefore(bar, bordersTab.firstChild); }
      if (bar.dataset.dekanKey !== borderKey || bar.childElementCount === 0) {
        bar.dataset.dekanKey = borderKey;
        bar.innerHTML = "";
        const title = document.createElement("div");
        title.className = "dekan-tier-bar-title";
        title.textContent = translate("Dekan — All ranks & prestige");
        bar.appendChild(title);
        RANKED_TIERS.forEach((tier) => { bar.appendChild(makeTierChip(tier, borderKey === "ranked:" + tier, null, () => saveIdentityPatch({ border: "ranked:" + tier }))); });
        PRESTIGE_LEVELS.forEach((lvl, idx) => { const val = "prestige:" + (idx + 1) + ":" + lvl; bar.appendChild(makeTierChip("Nv." + lvl, borderKey === val, null, () => saveIdentityPatch({ border: val }))); });
      }
    }
    const bannersTab = document.querySelector(".identity-customizer-tab-banners-component");
    if (bannersTab) {
      let bar = document.getElementById(BANNERS_BAR_ID);
      if (!bar) { bar = document.createElement("div"); bar.id = BANNERS_BAR_ID; bar.className = "dekan-tier-bar"; bannersTab.insertBefore(bar, bannersTab.firstChild); }
      if (bar.dataset.dekanKey !== bannerKey || bar.childElementCount === 0) {
        bar.dataset.dekanKey = bannerKey;
        bar.innerHTML = "";
        const title = document.createElement("div");
        title.className = "dekan-tier-bar-title";
        title.textContent = translate("Dekan — Rank banners");
        bar.appendChild(title);
        RANKED_TIERS.forEach((tier) => {
          const asset = "/lol-game-data/assets/ASSETS/Regalia/BannerSkins/" + tier.toLowerCase() + ".png";
          bar.appendChild(makeTierChip(tier, bannerKey === asset, asset, () => saveIdentityPatch({ banner: asset })));
        });
      }
    }
  }

  function unlockIdentityCustomizer() {
    installIdentityNetworkHook();
    if (!document.querySelector(".challenges-identity-customizer-root-component")) return;
    ensureCatalogsLoaded();
    renderDekanTierBars();
  }
