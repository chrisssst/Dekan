
  function setImgOverride(el, src) {
    if (!el || !src) return;
    const img = el.tagName === "IMG" ? el : el.querySelector && el.querySelector("img");
    if (img) {
      if (!img.dataset.dekanIdOrig) img.dataset.dekanIdOrig = img.getAttribute("src") || "";
      if (img.getAttribute("src") !== src) img.setAttribute("src", src);
      return;
    }
    const childBg = el.querySelector && el.querySelector("[style*='background']");
    const target = childBg || el;
    if (!target.dataset.dekanIdOrigBg) target.dataset.dekanIdOrigBg = target.style.backgroundImage || "";
    target.style.backgroundImage = "url('" + src + "')";
  }

  function setTextOverride(el, text) {
    if (!el || !text) return;
    if (!el.dataset.dekanIdOrigText) el.dataset.dekanIdOrigText = el.textContent || "";
    if (el.textContent !== text) el.textContent = text;
  }

  function extractBannerAssetFromCard(bannerEl) {
    const regEl = bannerEl.querySelector("lol-regalia-banner-v2-element");
    if (regEl) {
      const rank = (regEl.getAttribute("banner-rank") || "").toUpperCase();
      if (RANKED_TIERS.includes(rank)) {
        return "/lol-game-data/assets/ASSETS/Regalia/BannerSkins/" + rank.toLowerCase() + ".png";
      }
      try {
        const img = regEl.shadowRoot && regEl.shadowRoot.querySelector(".regalia-banner-asset-static-image");
        const src = img && img.getAttribute("src");
        if (src && (src.startsWith("/lol-game-data/assets/") || src.startsWith("/fe/lol-static-assets/"))) return src;
      } catch (_) {}
    }
    const img = bannerEl.querySelector("img[src]:not(.identity-customizer-banner-locked)");
    const src = img && img.getAttribute("src");
    if (src && !src.includes("locked") && (src.startsWith("/lol-game-data/assets/") || src.startsWith("/fe/lol-static-assets/"))) {
      return src;
    }
    return null;
  }

  function handleIdentityClick(ev) {
    if (!ev.target || !ev.target.closest) return;
    const rootEl = ev.target.closest(".challenges-identity-customizer-root-component");
    if (!rootEl) return;
    if (ev.target.closest(".dekan-tier-bar")) return;
    const bannerEl = ev.target.closest(".identity-customizer-banner");
    if (bannerEl) {
      const asset = extractBannerAssetFromCard(bannerEl);
      if (asset) saveIdentityPatch({ banner: asset });
    }
    const iconEl = ev.target.closest(".identity-customizer-icon, .icon-image-container");
    if (iconEl) {
      const img = iconEl.querySelector("img[src*='profile-icons/']") || (iconEl.tagName === "IMG" ? iconEl : null);
      const m = img && (img.getAttribute("src") || "").match(/profile-icons\/(\d+)\./);
      if (m && Number(m[1]) > 0) saveIdentityPatch({ icon_id: Number(m[1]) });
    }
    const titleEl = ev.target.closest(".identity-customizer-title-component");
    if (titleEl) {
      const nameEl = titleEl.querySelector(".title-name, .identity-customizer-title-name, .challenge-title-name");
      const txt = nameEl ? (nameEl.textContent || "").trim().slice(0, 64) : "";
      if (txt) saveIdentityPatch({ title: txt });
    }
    const tokEl = ev.target.closest(".identity-customizer-token-component");
    if (tokEl) {
      const img = tokEl.querySelector("img[src]");
      const src = img && img.getAttribute("src");
      if (src && src.startsWith("/")) {
        const cur = Array.isArray(state.identity && state.identity.tokens) ? state.identity.tokens.slice(0, 2) : [];
        cur.unshift(src);
        saveIdentityPatch({ tokens: cur.slice(0, 3) });
      }
    }
    defer(unlockIdentityCustomizer, 40);
    defer(unlockIdentityCustomizer, 180);
  }

  function applyCrestAttrs(crestEl, id) {
    if (!crestEl || !id) return;
    if (id.icon_id) {
      const iconUrl = "/lol-game-data/assets/v1/profile-icons/" + id.icon_id + ".jpg";
      if (crestEl.getAttribute("profile-icon") !== String(id.icon_id)) crestEl.setAttribute("profile-icon", String(id.icon_id));
      if (crestEl.getAttribute("profile-icon-url") !== iconUrl) crestEl.setAttribute("profile-icon-url", iconUrl);
    }
    if (id.border && id.border.startsWith("ranked:")) {
      const tier = id.border.slice(7);
      const div = APEX_TIERS.includes(tier) ? "NA" : "I";
      if (crestEl.getAttribute("crest-type") !== "ranked") crestEl.setAttribute("crest-type", "ranked");
      if (crestEl.getAttribute("ranked-tier") !== tier) crestEl.setAttribute("ranked-tier", tier);
      if (crestEl.getAttribute("ranked-division") !== div) crestEl.setAttribute("ranked-division", div);
    } else if (id.border && id.border.startsWith("prestige:")) {
      const parts = id.border.split(":");
      if (crestEl.getAttribute("crest-type") !== "prestige") crestEl.setAttribute("crest-type", "prestige");
      if (parts[1] && crestEl.getAttribute("prestige-crest-id") !== parts[1]) crestEl.setAttribute("prestige-crest-id", parts[1]);
      if (parts[2] && crestEl.getAttribute("crest-by-level") !== parts[2]) crestEl.setAttribute("crest-by-level", parts[2]);
      if (parts[2] && crestEl.getAttribute("summoner-level") !== parts[2]) crestEl.setAttribute("summoner-level", parts[2]);
    }
  }

  function applyBannerAttrs(bannerEl, id) {
    if (!bannerEl || !id || !id.banner) return;
    const m = id.banner.match(/BannerSkins\/([a-z]+)\.png$/i);
    const tier = m ? m[1].toUpperCase() : "";
    if (RANKED_TIERS.includes(tier)) {
      if (bannerEl.getAttribute("banner-id") !== "2") bannerEl.setAttribute("banner-id", "2");
      if (bannerEl.getAttribute("banner-type") !== "lastSeasonHighestRank") bannerEl.setAttribute("banner-type", "lastSeasonHighestRank");
      if (bannerEl.getAttribute("banner-rank") !== tier) bannerEl.setAttribute("banner-rank", tier);
    } else {
      if (bannerEl.getAttribute("banner-type") !== "blank") bannerEl.setAttribute("banner-type", "blank");
      try {
        const img = bannerEl.shadowRoot && bannerEl.shadowRoot.querySelector(".regalia-banner-asset-static-image");
        if (img && img.getAttribute("src") !== id.banner) img.setAttribute("src", id.banner);
      } catch (_) {}
    }
  }

  function applyRegaliaElement(el, id) {
    if (!el || !id) return;
    const tag = (el.tagName || "").toLowerCase();
    if (tag === "lol-regalia-identity-customizer-element") {
      applyCrestAttrs(el, id);
      applyBannerAttrs(el, id);
    } else if (tag === "lol-regalia-crest-v2-element") {
      applyCrestAttrs(el, id);
    } else if (tag === "lol-regalia-banner-v2-element") {
      applyBannerAttrs(el, id);
    }
    try {
      const sr = el.shadowRoot;
      if (sr) {
        const crest = sr.querySelector("lol-regalia-crest-v2-element");
        if (crest) applyCrestAttrs(crest, id);
        const banner = sr.querySelector("lol-regalia-banner-v2-element");
        if (banner) applyBannerAttrs(banner, id);
      }
    } catch (_) {}
  }

  function applyProfileRankedEmblem(id) {
    if (!id || !id.border || !id.border.startsWith("ranked:")) return;
    const prof = document.querySelector("lol-regalia-profile-v2-element");
    if (prof && (prof.classList.contains("is-searched") || prof.getAttribute("is-searched") === "true")) return;
    const comp = document.querySelector(".style-profile-ranked-component");
    if (!comp) return;
    const tier = id.border.slice(7);
    const div = APEX_TIERS.includes(tier) ? "NA" : "I";
    const emb = comp.querySelector("lol-regalia-emblem-element");
    if (emb) {
      if (emb.getAttribute("ranked-tier") !== tier) emb.setAttribute("ranked-tier", tier);
      if (emb.getAttribute("ranked-division") !== div) emb.setAttribute("ranked-division", div);
    }
    const sub = comp.querySelector(".style-profile-emblem-header-subtitle");
    setTextOverride(sub, formatTierLabel(tier));
  }

  function applyHovercardIdentity(id) {
    if (!id) return;
    const cards = document.querySelectorAll(".hover-card");
    cards.forEach((card) => {
      const reg = card.querySelector("lol-regalia-hovercard-v2-element");
      if (!reg) return;
      const sid = reg.getAttribute("summoner-id");
      const puuid = reg.getAttribute("puuid");
      const isMe = (localSummonerId && sid && String(sid) === String(localSummonerId))
        || (localPuuid && puuid && String(puuid) === String(localPuuid));
      if (!isMe) return;
      applyRegaliaElement(reg, id);
      if (id.title) {
        const tEl = card.querySelector(".hover-card-title");
        setTextOverride(tEl, id.title);
      }
      if (id.border && id.border.startsWith("ranked:")) {
        const tier = id.border.slice(7);
        const miniUrl = "/fe/lol-static-assets/images/ranked-mini-crests/" + tier.toLowerCase() + ".svg";
        const rankRow = card.querySelector(".hover-card-ranked-description .hover-card-rank");
        if (rankRow) {
          const img = rankRow.querySelector(".hover-card-rank-image");
          setImgOverride(img, miniUrl);
          const label = formatTierLabel(tier);
          for (const node of Array.from(rankRow.childNodes)) {
            if (node.nodeType === 3 && (node.nodeValue || "").trim()) {
              const cur = node.nodeValue.trim();
              const paren = cur.indexOf("(");
              const suffix = paren >= 0 ? (" " + cur.slice(paren)) : "";
              const next = " " + label + suffix;
              if (node.nodeValue !== next) node.nodeValue = next;
              break;
            }
          }
        }
      }
    });
  }

  function applyIdentityToRoot(root, id) {
    if (!root || !id) return;
    if (id.icon_id) {
      const iconSrc = "/lol-game-data/assets/v1/profile-icons/" + id.icon_id + ".jpg";
      root.querySelectorAll("img[src*='profile-icons/']").forEach((img) => setImgOverride(img, iconSrc));
    }
    if (id.title) {
      const titleEl = root.querySelector(".player-title, .summoner-title, .challenge-title, .video-preview-title");
      setTextOverride(titleEl, id.title);
    }
    if (id.banner) {
      const bannerImg = root.querySelector("img[src*='regalia']:not(.identity-customizer-banner-locked), .video-preview-banner img");
      setImgOverride(bannerImg, id.banner);
    }
    if (Array.isArray(id.tokens) && id.tokens.length) {
      const imgs = root.querySelectorAll(".challenge-token img, .tokens-container img, .challenge-banner-token-component img");
      imgs.forEach((img, idx) => {
        const tok = id.tokens[idx];
        if (tok && tok.startsWith("/")) setImgOverride(img, tok);
      });
    }
    const sel = "lol-regalia-identity-customizer-element, lol-regalia-profile-v2-element, lol-regalia-parties-v2-element, lol-regalia-hovercard-v2-element, lol-regalia-crest-v2-element, lol-regalia-banner-v2-element";
    root.querySelectorAll(sel).forEach((el) => applyRegaliaElement(el, id));
    if (root.matches && root.matches(sel)) applyRegaliaElement(root, id);
  }

  function applyLocalBackdropDom() {
    const bg = getStoredBackdrop();
    if (!bg || !bg.backdropImage) return;
    const prof = document.querySelector("lol-regalia-profile-v2-element");
    if (prof && (prof.classList.contains("is-searched") || prof.getAttribute("is-searched") === "true")) return;
    const box = document.querySelector(".style-profile-background-image");
    if (!box) return;
    const target = box.querySelector("[style*='background-image'], .uikit-background-switcher-item") || box;
    const want = "url(\"" + bg.backdropImage + "\")";
    if (!target.dataset.dekanIdOrigBg) target.dataset.dekanIdOrigBg = target.style.backgroundImage || "";
    if (target.style.backgroundImage !== want) target.style.backgroundImage = want;
  }

  function applyLocalIdentityDom() {
    applyLocalBackdropDom();
    const id = getActiveIdentity();
    if (!id) return;
    const preview = document.querySelector(".challenges-identity-customizer-banner-container, .identity-customizer-modal .regalia-profile-preview");
    if (preview) applyIdentityToRoot(preview, id);
    const customizerRegalia = document.querySelector("lol-regalia-identity-customizer-element");
    if (customizerRegalia) applyRegaliaElement(customizerRegalia, id);
    const profileRegalia = document.querySelector("lol-regalia-profile-v2-element");
    if (profileRegalia && !profileRegalia.classList.contains("is-searched") && profileRegalia.getAttribute("is-searched") !== "true") {
      applyRegaliaElement(profileRegalia, id);
    }
    applyProfileRankedEmblem(id);
    applyHovercardIdentity(id);
    const socialIcon = document.querySelector(".lol-social-identity img[src*='profile-icons/']");
    if (socialIcon && id.icon_id) setImgOverride(socialIcon, "/lol-game-data/assets/v1/profile-icons/" + id.icon_id + ".jpg");
    const localLobby = document.querySelector(".parties-player-card.local-player, .lobby-member.is-local");
    if (localLobby) applyIdentityToRoot(localLobby, id);
  }

  function applyPeerIdentitiesDom() {
    const peers = state.catalog && state.catalog.party && state.catalog.party.peer_identities;
    if (!Array.isArray(peers) || peers.length === 0) return;
    const byName = new Map();
    for (const pair of peers) {
      if (Array.isArray(pair) && pair.length >= 2 && pair[0] && pair[1]) {
        byName.set(String(pair[0]).toLowerCase(), pair[1]);
      }
    }
    if (byName.size === 0) return;
    const cards = document.querySelectorAll(".parties-player-card, .lobby-member, .lol-social-roster-item");
    cards.forEach((card) => {
      const nameEl = card.querySelector(".parties-player-name, .summoner-name, .name");
      const raw = nameEl ? (nameEl.textContent || "").trim().toLowerCase() : "";
      if (!raw) return;
      const peerId = byName.get(raw) || byName.get(raw.split("#")[0]);
      if (!peerId) return;
      applyIdentityToRoot(card, peerId);
      if (nameEl && !card.querySelector(".dekan-party-badge")) {
        const badge = document.createElement("span");
        badge.className = "dekan-party-badge";
        badge.textContent = "PARTY";
        nameEl.appendChild(badge);
      }
    });
  }

  function syncPartyJoinButton() {
    const party = state.catalog && state.catalog.party;
    const existing = document.getElementById(PARTY_BTN_ID);
    const joinable = party ? (party.joinable_friends || 0) : 0;
    const connected = party ? (party.connected_members || 0) : 0;
    if (joinable === 0 && connected < 2) {
      if (existing) existing.style.display = "none";
      return;
    }
    const nav = document.querySelector(".right-nav-menu");
    if (!nav) return;
    let btn = existing;
    if (!btn) {
      btn = document.createElement("button");
      btn.id = PARTY_BTN_ID;
      btn.type = "button";
      const dot = document.createElement("span");
      dot.className = "dekan-party-dot";
      const span = document.createElement("span");
      span.className = "dekan-party-label";
      btn.appendChild(dot);
      btn.appendChild(span);
      btn.onclick = (ev) => {
        ev.stopPropagation(); ev.preventDefault();
        if (state.catalog && state.catalog.party && state.catalog.party.joinable_friends > 0) {
          state.queue.push({ type: "join_lobby_party" });
        } else {
          state.popoverTab = "party";
          renderNavPopover(document.getElementById(NAV_BTN_ID) || btn);
        }
      };
      const navBtn = document.getElementById(NAV_BTN_ID);
      if (navBtn && navBtn.parentNode === nav) nav.insertBefore(btn, navBtn);
      else nav.insertBefore(btn, nav.firstChild);
    }
    const span = btn.querySelector(".dekan-party-label");
    if (joinable > 0) {
      span.textContent = translate("Join Party (") + joinable + ")";
    } else {
      span.textContent = translate("Party (") + connected + ")";
    }
    btn.style.display = "inline-flex";
  }

  function syncIdentityAndParty() {
    if (state.disposed) return;
    ensureIdentityStyle();
    unlockIdentityCustomizer();
    applyLocalIdentityDom();
    applyPeerIdentitiesDom();
    syncPartyJoinButton();
    if (!state.hoverHandler) {
      state.hoverHandler = (ev) => {
        if (!ev.target || !ev.target.closest) return;
        if (ev.target.closest(".lol-social-identity, .lol-social-roster-item")) {
          const id = getActiveIdentity();
          if (id) defer(() => applyHovercardIdentity(id), 35);
        }
      };
      document.addEventListener("mouseover", state.hoverHandler, true);
    }
    if (!state.idWatchTimer) {
      state.idWatchTimer = setInterval(() => {
        if (document.hidden) return;
        if (getActiveIdentity() || getStoredBackdrop() || document.querySelector(".challenges-identity-customizer-root-component")) {
          unlockIdentityCustomizer();
          applyLocalIdentityDom();
          applyPeerIdentitiesDom();
        }
      }, 800);
    }
  }

  function cleanupIdentityAndParty() {
    if (state.idWatchTimer) { clearInterval(state.idWatchTimer); state.idWatchTimer = null; }
    if (state.hoverHandler) { document.removeEventListener("mouseover", state.hoverHandler, true); state.hoverHandler = null; }
    if (state.origFetch) { window.fetch = state.origFetch; state.origFetch = null; }
    const XHR = window.XMLHttpRequest;
    if (XHR && XHR.prototype) {
      if (state.origXhrOpen) { XHR.prototype.open = state.origXhrOpen; state.origXhrOpen = null; }
      if (state.origXhrSend) { XHR.prototype.send = state.origXhrSend; state.origXhrSend = null; }
      if (state.origXhrTextDesc) {
        try { Object.defineProperty(XHR.prototype, "responseText", state.origXhrTextDesc); } catch (_) {}
        state.origXhrTextDesc = null;
      }
      if (state.origXhrRespDesc) {
        try { Object.defineProperty(XHR.prototype, "response", state.origXhrRespDesc); } catch (_) {}
        state.origXhrRespDesc = null;
      }
    }
    state.idNetHooked = false;
    const st = document.getElementById(ID_STYLE_ID);
    if (st) st.remove();
    const b1 = document.getElementById(BORDERS_BAR_ID);
    if (b1) b1.remove();
    const b2 = document.getElementById(BANNERS_BAR_ID);
    if (b2) b2.remove();
    const btn = document.getElementById(PARTY_BTN_ID);
    if (btn) btn.remove();
    document.querySelectorAll(".dekan-party-badge").forEach((el) => el.remove());
    document.querySelectorAll("[data-dekan-id-orig]").forEach((el) => {
      if (el.dataset.dekanIdOrig) el.setAttribute("src", el.dataset.dekanIdOrig);
      delete el.dataset.dekanIdOrig;
    });
    document.querySelectorAll("[data-dekan-id-orig-bg]").forEach((el) => {
      el.style.backgroundImage = el.dataset.dekanIdOrigBg || "";
      delete el.dataset.dekanIdOrigBg;
    });
    document.querySelectorAll("[data-dekan-id-orig-text]").forEach((el) => {
      el.textContent = el.dataset.dekanIdOrigText || "";
      delete el.dataset.dekanIdOrigText;
    });
  }
