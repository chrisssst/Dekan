const TURKISH = {
  "Chromas / Forms": "Renkler / Biçimler",
  "Lv.": "Sv.",
  "PARTY": "PARTİ",
  "IRON": "DEMİR", "BRONZE": "BRONZ", "SILVER": "GÜMÜŞ", "GOLD": "ALTIN",
  "PLATINUM": "PLATİN", "EMERALD": "ZÜMRÜT", "DIAMOND": "ELMAS",
  "MASTER": "USTALIK", "GRANDMASTER": "ÜSTATLIK", "CHALLENGER": "ŞAMPİYONLUK",
  "Home": "Ana sayfa",
  "Mods": "Modlar",
  "Party": "Parti",
  "Support": "Destek",
  "Enabled": "Etkin",
  "In-Client mode active — pick skins directly in the carousel.": "İstemci içi mod etkin — kostümünü doğrudan karuselden seç.",
  "In-Client mode active — ready for League, TFT & Customize Identity.": "İstemci içi mod etkin — League, TFT ve kimlik özelleştirme hazır.",
  "Random skin": "Rastgele kostüm",
  "Toggle skin preset": "Kostüm ön ayarını sabitle / kaldır",
  "Enjoying Dekan? Share it with your friends and star the repository on GitHub to support the project.": "Dekan’ı beğendin mi? Arkadaşlarınla paylaş ve desteklemek için GitHub reposuna yıldız ver.",
  "Star Dekan on GitHub": "GitHub’da Dekan’a yıldız ver",
  "Map": "Harita",
  "Font": "Yazı tipi",
  "Announcer": "Anons",
  "Skin mod": "Kostüm modu",
  "Others": "Diğerleri",
  "No custom mods installed (maps, fonts, announcers).": "Kurulu özel mod yok (harita, yazı tipi, anons).",
  "Open mods folder": "Mod klasörünü aç",
  "Join a lobby with friends running Dekan to connect to Party automatically.": "Partiye otomatik bağlanmak için Dekan kullanan arkadaşlarınla bir lobiye katıl.",
  "Reset Identity": "Kimliği sıfırla",
  "Dekan runs natively inside the client with zero Pengu Loader. Share with friends and star the repo!": "Dekan, Pengu Loader olmadan istemci içinde çalışır. Arkadaşlarınla paylaş ve repoya yıldız ver!",
  "Copy link for friends": "Bağlantıyı arkadaşların için kopyala",
  "Link copied!": "Bağlantı kopyalandı!",
  "Dekan — All ranks & prestige": "Dekan — Tüm dereceler ve prestij",
  "Dekan — Rank banners": "Dekan — Derece sancakları",
  "Join Party (": "Partiye katıl (",
  "Party (": "Parti ("
};
function translate(value) {
  const language = (window.__dekanLanguage || "en").slice(0, 2).toLowerCase();
  if (language !== "tr") return value;
  if (TURKISH[value]) return TURKISH[value];
  return value.replace(/^Join Lobby Party \((\d+)\)$/, "Lobi partisine katıl ($1)")
    .replace(/^Party active \((\d+) players\) — skins and identity synced with E2EE\.$/, "Parti etkin ($1 oyuncu) — kostümler ve kimlik uçtan uca şifrelemeyle eşitlendi.");
}
