
  function cloneModSel() {
    const s = (state.catalog && state.catalog.mods && state.catalog.mods.selection) || {};
    return { skin: s.skin || null, map: s.map || null, font: s.font || null, announcer: s.announcer || null, others: Array.isArray(s.others) ? s.others.slice() : [] };
  }
  function toggleModSlot(slot, id, anchor) {
    const next = cloneModSel();
    if (slot === "others") {
      const idx = next.others.indexOf(id);
      if (idx >= 0) next.others.splice(idx, 1); else next.others.push(id);
    } else {
      next[slot] = next[slot] === id ? null : id;
    }
    if (state.catalog && state.catalog.mods) state.catalog.mods.selection = next;
    state.queue.push({ type: "set_mods", selection: next });
    renderNavPopover(anchor);
  }
  function renderNavPopover(anchor) {
    closeFlyout();
    let pop = document.getElementById(POPOVER_ID);
    if (!pop) {
      try {
        pop = document.createElement("lol-uikit-flyout-frame");
        pop.setAttribute("orientation", "bottom");
        pop.setAttribute("animated", "false");
        pop.setAttribute("show", "true");
      } catch (_) {
        pop = document.createElement("div");
      }
      pop.id = POPOVER_ID;
      document.body.appendChild(pop);
    }
    pop.innerHTML = "";
    let content;
    try { content = document.createElement("lc-flyout-content"); } catch (_) { content = document.createElement("div"); }
    const frame = document.createElement("div");
    frame.className = "dekan-pop-frame";
    const head = document.createElement("div");
    head.className = "dekan-pop-head";
    const brand = document.createElement("span");
    brand.className = "dekan-pop-brand";
    brand.textContent = "DEKAN";
    head.appendChild(brand);
    const tabs = document.createElement("div");
    tabs.className = "dekan-pop-tabs";
    const tabDefs = [
      ["home", translate("Home")],
      ["mods", translate("Mods")],
      ["party", translate("Party")],
      ["support", translate("Support")],
    ];
    if (!tabDefs.some(([k]) => k === state.popoverTab)) state.popoverTab = "home";
    for (const [key, label] of tabDefs) {
      const tb = document.createElement("button");
      tb.type = "button";
      tb.className = "dekan-pop-tab" + (state.popoverTab === key ? " active" : "");
      tb.textContent = label;
      tb.onclick = (ev) => { ev.stopPropagation(); state.popoverTab = key; renderNavPopover(anchor); };
      tabs.appendChild(tb);
    }
    head.appendChild(tabs);
    frame.appendChild(head);
    const body = document.createElement("div");
    body.className = "dekan-pop-body";
    const party = state.catalog && state.catalog.party;

    if (state.popoverTab === "home") {
      const status = document.createElement("div");
      status.className = "dekan-pop-empty";
      status.textContent = (state.catalog && state.catalog.champion_id > 0)
        ? translate("In-Client mode active — pick skins directly in the carousel.")
        : translate("In-Client mode active — ready for League, TFT & Customize Identity.");
      body.appendChild(status);
      if (state.catalog && state.catalog.champion_id > 0) {
        const randBtn = document.createElement("button");
        randBtn.type = "button";
        randBtn.className = "dekan-pop-btn primary";
        randBtn.textContent = translate("Random skin");
        randBtn.onclick = (ev) => { ev.stopPropagation(); state.suppressBaseUntil = Date.now() + 8000; state.queue.push({ type: "random" }); closePopover(); };
        body.appendChild(randBtn);
        const pinBtn = document.createElement("button");
        pinBtn.type = "button";
        pinBtn.className = "dekan-pop-btn";
        pinBtn.textContent = translate("Toggle skin preset");
        pinBtn.onclick = (ev) => { ev.stopPropagation(); state.queue.push({ type: "toggle_preset" }); closePopover(); };
        body.appendChild(pinBtn);
      }
      const shareCard = document.createElement("div");
      shareCard.className = "dekan-pop-empty";
      shareCard.style.cssText = "border:1px solid #463714;background:#0a1420;padding:8px 10px;color:#cdbe91;";
      shareCard.textContent = translate("Enjoying Dekan? Share it with your friends and star the repository on GitHub to support the project.");
      body.appendChild(shareCard);
      const starBtn = document.createElement("button");
      starBtn.type = "button";
      starBtn.className = "dekan-pop-btn primary";
      starBtn.textContent = translate("Star Dekan on GitHub");
      starBtn.onclick = (ev) => { ev.stopPropagation(); window.open("https://github.com/chrisssst/Dekan", "_blank"); closePopover(); };
      body.appendChild(starBtn);
    } else if (state.popoverTab === "mods") {
      const mods = state.catalog && state.catalog.mods;
      const groups = [
        ["map", translate("Map")],
        ["font", translate("Font")],
        ["announcer", translate("Announcer")],
        ["skin", translate("Skin mod")],
        ["others", translate("Others")],
      ];
      let any = false;
      const sel = cloneModSel();
      for (const [slot, title] of groups) {
        const list = mods && Array.isArray(mods[slot]) ? mods[slot] : [];
        if (list.length === 0) continue;
        any = true;
        const wrap = document.createElement("div");
        const h = document.createElement("div");
        h.className = "dekan-mod-group-title";
        h.textContent = title;
        wrap.appendChild(h);
        const row = document.createElement("div");
        row.className = "dekan-chip-row";
        for (const item of list) {
          const active = slot === "others" ? sel.others.includes(item.id) : sel[slot] === item.id;
          const chip = document.createElement("button");
          chip.type = "button";
          chip.className = "dekan-chip" + (active ? " active" : "");
          chip.textContent = item.name;
          chip.onclick = (ev) => { ev.stopPropagation(); toggleModSlot(slot, item.id, anchor); };
          row.appendChild(chip);
        }
        wrap.appendChild(row);
        body.appendChild(wrap);
      }
      if (!any) {
        const empty = document.createElement("div");
        empty.className = "dekan-pop-empty";
        empty.textContent = translate("No custom mods installed (maps, fonts, announcers).");
        body.appendChild(empty);
      }
      const folderBtn = document.createElement("button");
      folderBtn.type = "button";
      folderBtn.className = "dekan-pop-btn";
      folderBtn.textContent = translate("Open mods folder");
      folderBtn.onclick = (ev) => { ev.stopPropagation(); state.queue.push({ type: "open_mods_folder" }); closePopover(); };
      body.appendChild(folderBtn);
    } else if (state.popoverTab === "party") {
      if (party && party.joinable_friends > 0) {
        const joinBtn = document.createElement("button");
        joinBtn.type = "button";
        joinBtn.className = "dekan-pop-btn primary";
        joinBtn.textContent = translate("Join Lobby Party (" + party.joinable_friends + ")");
        joinBtn.onclick = (ev) => { ev.stopPropagation(); state.queue.push({ type: "join_lobby_party" }); closePopover(); };
        body.appendChild(joinBtn);
      } else if (party && party.connected_members >= 2) {
        const info = document.createElement("div");
        info.className = "dekan-pop-empty";
        info.textContent = translate("Party active (" + party.connected_members + " players) — skins and identity synced with E2EE.");
        body.appendChild(info);
      } else {
        const info = document.createElement("div");
        info.className = "dekan-pop-empty";
        info.textContent = translate("Join a lobby with friends running Dekan to connect to Party automatically.");
        body.appendChild(info);
      }
      if (getActiveIdentity() || getStoredBackdrop()) {
        const resetIdBtn = document.createElement("button");
        resetIdBtn.type = "button";
        resetIdBtn.className = "dekan-pop-btn";
        resetIdBtn.textContent = translate("Reset Identity");
        resetIdBtn.onclick = (ev) => { ev.stopPropagation(); state.identity = null; setStoredBackdrop(null); try { window.localStorage.removeItem(ID_STORAGE_KEY); } catch (_) {} state.queue.push({ type: "set_identity", loadout: null }); cleanupIdentityAndParty(); closePopover(); };
        body.appendChild(resetIdBtn);
      }
    } else {
      const tip = document.createElement("div");
      tip.className = "dekan-pop-empty";
      tip.textContent = translate("Dekan runs natively inside the client with zero Pengu Loader. Share with friends and star the repo!");
      body.appendChild(tip);
      const copyBtn = document.createElement("button");
      copyBtn.type = "button";
      copyBtn.className = "dekan-pop-btn primary";
      copyBtn.textContent = translate("Copy link for friends");
      copyBtn.onclick = (ev) => {
        ev.stopPropagation();
        try { navigator.clipboard.writeText("https://github.com/chrisssst/Dekan"); } catch (_) {}
        copyBtn.textContent = translate("Link copied!");
      };
      body.appendChild(copyBtn);
      const discordBtn = document.createElement("button");
      discordBtn.type = "button";
      discordBtn.className = "dekan-pop-btn";
      discordBtn.textContent = "Discord";
      discordBtn.onclick = (ev) => { ev.stopPropagation(); window.open("https://discord.gg/dekan", "_blank"); closePopover(); };
      body.appendChild(discordBtn);
    }

    frame.appendChild(body);
    content.appendChild(frame);
    pop.appendChild(content);
    const rect = anchor && anchor.getBoundingClientRect ? anchor.getBoundingClientRect() : null;
    if (rect && rect.width > 0) {
      pop.style.left = Math.max(12, Math.min(window.innerWidth - 340, Math.round(rect.left + rect.width / 2 - 166))) + "px";
      pop.style.top = Math.round(rect.bottom + 4) + "px";
    } else {
      pop.style.right = "18px";
      pop.style.top = "82px";
    }
  }
  function ensureNavButton() {
    const nav = document.querySelector(".right-nav-menu");
    if (!nav || document.getElementById(NAV_BTN_ID)) return;
    const item = document.createElement("lol-uikit-navigation-item");
    item.id = NAV_BTN_ID;
    item.className = "main-navigation-menu-item menu_item_dekan ember-view";
    item.title = "Dekan";
    const wrap = document.createElement("div");
    wrap.className = "menu-item-icon-wrapper";
    const glow = document.createElement("div");
    glow.className = "menu-item-glow";
    const icon = document.createElement("div");
    icon.className = "menu-item-icon dekan-nav-ico";
    icon.style.webkitMaskImage = "url(" + DEKAN_ICON_URI + ")";
    wrap.appendChild(glow);
    wrap.appendChild(icon);
    item.appendChild(wrap);
    item.onclick = (ev) => {
      ev.stopPropagation(); ev.preventDefault();
      if (document.getElementById(POPOVER_ID)) { closePopover(); }
      else { item.classList.add("open"); renderNavPopover(item); }
    };
    const sep = document.createElement("div");
    sep.className = "right-nav-vertical-rule dekan-nav-sep";
    const first = nav.firstChild || null;
    nav.insertBefore(item, first);
    nav.insertBefore(sep, item.nextSibling);
  }
