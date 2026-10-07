use crate::dekan_data_dir;

pub(crate) fn run_client_probe() {
    use dekan_platform::client_window::{
        ClientWindowState, client_window_state, overlay_placement,
    };

    match client_window_state() {
        ClientWindowState::Absent => {
            println!("cliente: AUSENTE (nenhuma janela de classe RCLIENT)");
        }
        ClientWindowState::Hidden => {
            println!("cliente: OCULTO ou MINIMIZADO (rect nao utilizavel; overlay fica escondido)");
        }
        ClientWindowState::Visible(rect) => {
            println!(
                "cliente: VISIVEL em {},{} -> {},{}  ({}x{})",
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                rect.width(),
                rect.height()
            );
            let placement = overlay_placement(rect, 360, 520, 16);
            println!(
                "overlay: {},{} -> {},{}  ({}x{})",
                placement.left,
                placement.top,
                placement.right,
                placement.bottom,
                placement.width(),
                placement.height()
            );
        }
    }
}

pub(crate) fn run_overlay_demo() {
    use dekan_platform::client_window::client_window_state;
    use dekan_platform::overlay_window::{OverlayWindow, track_once};
    use std::time::{Duration, Instant};

    let (overlay, _commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] nao foi possivel criar o overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();

    println!("Overlay criado. Acompanhando a janela do cliente por 30s.");
    println!("Restaure o cliente do League se ele estiver minimizado.");

    let started = Instant::now();
    let mut last = String::new();

    while started.elapsed() < Duration::from_secs(30) {
        let placement = track_once(&controller, true);
        let now = match (client_window_state(), placement) {
            (_, Some(rect)) => format!(
                "VISIVEL - overlay em {},{} ({}x{})",
                rect.left,
                rect.top,
                rect.width(),
                rect.height()
            ),
            (state, None) => format!("{state:?} - overlay escondido"),
        };
        if now != last {
            println!("[{:>5.1}s] {now}", started.elapsed().as_secs_f32());
            last = now;
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    println!("Fim da demo; overlay encerrado.");
}

pub(crate) fn run_library_probe(champion_id: Option<u32>) {
    use dekan_core::library::{champions_with_content, scan_champion};

    let data = dekan_data_dir();
    let roots = [data.join("library"), data.join("skins")];

    let Some(root) = roots.iter().find(|p| p.is_dir()) else {
        println!("nenhuma biblioteca encontrada em:");
        for r in &roots {
            println!("  {}", r.display());
        }
        return;
    };

    println!("biblioteca: {}", root.display());

    match champion_id {
        Some(id) => {
            let library = scan_champion(root, id);
            println!(
                "campeao {id}: {} skins, {} pacotes instalaveis",
                library.skins.len(),
                library.package_count()
            );
            for skin in library.skins.iter().take(8) {
                println!(
                    "  skin {} ({} chromas){}",
                    skin.id,
                    skin.chromas.len(),
                    if skin.chromas.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " -> {}",
                            skin.chromas
                                .iter()
                                .map(|c| c.id.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                );
            }
            if library.skins.len() > 8 {
                println!("  ... e mais {} skins", library.skins.len() - 8);
            }
        }
        None => {
            let found = champions_with_content(root);
            let total: usize = found.values().sum();
            println!(
                "{} campeoes com conteudo, {total} pacotes instalaveis no total",
                found.len()
            );
        }
    }
}

pub(crate) fn run_catalog_demo(champion_id: u32) {
    use dekan_app::catalog::{load_catalog, resolve_library_root};
    use dekan_platform::client_window::client_window_state;
    use dekan_platform::overlay_window::{OverlayWindow, track_once};
    use std::time::{Duration, Instant};

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("[ERRO] runtime: {e}");
            std::process::exit(1);
        }
    };

    let configured = dekan_data_dir().join("library");
    let root = resolve_library_root(&configured);
    println!("biblioteca: {}", root.display());

    let catalog = runtime.block_on(load_catalog(root, champion_id));
    println!(
        "catalogo: {} ({}) - {} skins, {} entradas",
        catalog.champion_name,
        catalog.champion_id,
        catalog.skins.len(),
        catalog.entry_count()
    );
    for skin in catalog.skins.iter().take(6) {
        println!(
            "  {} [{}]{}",
            skin.name,
            skin.id,
            if skin.chromas.is_empty() {
                String::new()
            } else {
                format!(" + {} chromas", skin.chromas.len())
            }
        );
    }

    let (overlay, mut commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();
    controller.set_catalog(catalog.clone());

    println!("Overlay com o catalogo real. 45s. Restaure o cliente se estiver minimizado.");
    let started = Instant::now();
    let mut last = String::new();
    while started.elapsed() < Duration::from_secs(45) {
        let placement = track_once(&controller, true);
        let now = match (client_window_state(), placement) {
            (_, Some(rect)) => format!("VISIVEL em {},{}", rect.left, rect.top),
            (state, None) => format!("{state:?} - escondido"),
        };
        if now != last {
            println!("[{:>5.1}s] {now}", started.elapsed().as_secs_f32());
            last = now;
        }

        while let Ok(command) = commands.try_recv() {
            match command {
                dekan_core::overlay::OverlayCommand::Select { id } => {
                    match catalog.resolve_target(id) {
                        Some(target) => println!(
                            "  clique -> alvo: campeao {} skin {} chroma {:?} pacote {}",
                            target.champion_id,
                            target.skin_id,
                            target.chroma_id,
                            target.package_entry_id()
                        ),
                        None => println!("  clique -> id {id} nao esta no catalogo (recusado)"),
                    }
                }
                dekan_core::overlay::OverlayCommand::Clear => {
                    println!("  clique -> selecao limpa")
                }
                dekan_core::overlay::OverlayCommand::SetMods { selection } => {
                    println!("  mods -> {selection:?}")
                }
                dekan_core::overlay::OverlayCommand::OpenModsFolder => {
                    println!("  mods -> abrir pasta")
                }
                dekan_core::overlay::OverlayCommand::Random => println!("  dado -> sortear skin"),
                dekan_core::overlay::OverlayCommand::ImportMod { category } => {
                    println!("  mods -> importar em {category:?}")
                }
                dekan_core::overlay::OverlayCommand::ChromaPreview { id } => {
                    println!("  hover -> preview do chroma {id}")
                }
                dekan_core::overlay::OverlayCommand::FocusChampion { id } => {
                    println!("  sala -> mostrar campeao {id}")
                }
                other => println!("  presets -> {other:?}"),
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("Fim da demo.");
}

pub(crate) fn run_ipc_probe(champion_id: u32) {
    use dekan_app::catalog::{load_catalog, resolve_library_root};
    use dekan_core::overlay::OverlayCommand;
    use dekan_platform::overlay_window::OverlayWindow;
    use std::time::{Duration, Instant};

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("[ERRO] runtime: {e}");
            std::process::exit(1);
        }
    };

    let configured = dekan_data_dir().join("library");
    let root = resolve_library_root(&configured);
    let catalog = runtime.block_on(load_catalog(root, champion_id));

    let Some(first_skin) = catalog.skins.first().cloned() else {
        eprintln!("[ERRO] campeao {champion_id} nao tem nenhuma skin na biblioteca local");
        std::process::exit(1);
    };
    let first_chroma = catalog
        .skins
        .iter()
        .find_map(|skin| skin.chromas.first().map(|c| c.id));

    let (overlay, mut commands) = match OverlayWindow::spawn() {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("[ERRO] overlay: {e}");
            std::process::exit(1);
        }
    };
    let controller = overlay.controller();
    controller.set_catalog(catalog.clone());
    println!(
        "catalogo carregado: {} ({}) - {} skins",
        catalog.champion_name,
        catalog.champion_id,
        catalog.skins.len()
    );

    std::thread::sleep(Duration::from_millis(600));

    let mut expected: Vec<u32> = vec![first_skin.id];
    controller.click(first_skin.id);
    if let Some(chroma_id) = first_chroma {
        println!("clicando skin {} e chroma {chroma_id}", first_skin.id);
        expected.push(chroma_id);
        std::thread::sleep(Duration::from_millis(300));
        controller.click(chroma_id);
    } else {
        println!("clicando skin {} (campeao sem chromas)", first_skin.id);
    }

    let mut received: Vec<u32> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while received.len() < expected.len() && Instant::now() < deadline {
        match commands.try_recv() {
            Ok(OverlayCommand::Select { id }) => {
                match catalog.resolve_target(id) {
                    Some(target) => println!(
                        "  recebido no Rust: id {id} -> campeao {} skin {} chroma {:?} pacote {}",
                        target.champion_id,
                        target.skin_id,
                        target.chroma_id,
                        target.package_entry_id()
                    ),
                    None => println!("  recebido no Rust: id {id} NAO esta no catalogo"),
                }
                received.push(id);
            }
            Ok(OverlayCommand::Clear) => println!("  recebido no Rust: selecao limpa"),
            Ok(OverlayCommand::SetMods { selection }) => {
                println!("  recebido no Rust: mods {selection:?}")
            }
            Ok(OverlayCommand::OpenModsFolder) => {
                println!("  recebido no Rust: abrir pasta de mods")
            }
            Ok(OverlayCommand::Random) => println!("  recebido no Rust: sortear skin"),
            Ok(OverlayCommand::ImportMod { category }) => {
                println!("  recebido no Rust: importar mod em {category:?}")
            }
            Ok(OverlayCommand::ChromaPreview { id }) => {
                println!("  recebido no Rust: preview do chroma {id}")
            }
            Ok(OverlayCommand::FocusChampion { id }) => {
                println!("  recebido no Rust: mostrar campeao {id} da sala")
            }
            Ok(other) => println!("  recebido no Rust: {other:?}"),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }

    drop(overlay);

    if received == expected {
        println!("[OK] o clique atravessou a fronteira JS -> Rust: {received:?}");
    } else {
        eprintln!("[FALHA] esperado {expected:?}, recebido {received:?}");
        std::process::exit(1);
    }
}
