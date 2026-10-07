use crate::{dekan_tools_dir, game_dir};

pub(crate) fn run_classic_probe(args: &[String]) {
    use dekan_classic::generator::{ClassicChampion, jade_characters, main_character, slots_for};

    let aliases: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    let Some(game) = game_dir() else {
        return;
    };
    let tools = dekan_tools_dir();
    let table = tools.join("hashes.game.txt");
    let cache = std::env::temp_dir().join("dekan_classic_probe_characters.json");
    let known = jade_characters(&table, &cache);
    println!("jogo: {}", game.display());
    println!("personagens jade_* na tabela de hashes: {}", known.len());

    let staging = std::env::temp_dir().join("dekan_classic_probe_mods");
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder may not exist
    if let Err(e) = std::fs::create_dir_all(&staging) {
        println!("pasta temporaria indisponivel: {e}");
        return;
    }

    for alias in aliases {
        let alias = alias.as_str();
        println!("\n== {alias}");
        let champion = match ClassicChampion::open(&game, alias) {
            Ok(champion) => champion,
            Err(e) => {
                println!("  nao abriu: {e}");
                continue;
            }
        };
        let regular = alias.to_ascii_lowercase();
        let jade = main_character(alias);
        let regular_numbers = champion.skin_numbers(&regular, 1000);
        let jade_numbers = champion.skin_numbers(&jade, 1000);
        let present = champion.present_characters(&known);
        println!("  personagens Classic presentes: {present:?}");

        let started = std::time::Instant::now();
        let from_bins = champion.jade_names_in_bins();
        let present_from_bins = champion.present_characters(&from_bins);
        println!(
            "  pelos .bin (sem tabela, {} ms): nomes {:?} -> presentes {:?} {}",
            started.elapsed().as_millis(),
            from_bins,
            present_from_bins,
            if present_from_bins == present {
                "IGUAL"
            } else {
                "DIFERENTE"
            }
        );

        println!(
            "  {regular}: {} numeros {:?}",
            regular_numbers.len(),
            regular_numbers
        );
        println!(
            "  {jade}: {} numeros {:?}",
            jade_numbers.len(),
            jade_numbers
        );
        let only_regular: Vec<u32> = regular_numbers
            .iter()
            .copied()
            .filter(|n| !jade_numbers.contains(n))
            .collect();
        println!("  existem no normal e NAO no Classic: {only_regular:?}");

        let mut built = 0usize;
        let mut failed = Vec::new();
        for number in jade_numbers.iter().copied().filter(|n| *n != 0) {
            match champion.build_mod(number, &slots_for(None), &known, &staging) {
                Ok(_) => built += 1,
                Err(e) => failed.push(format!("{number}: {e}")),
            }
        }
        println!(
            "  mods Classic gerados dos bins reais: {built} ok, {} falharam {failed:?}",
            failed.len()
        );
    }
    let _ = std::fs::remove_dir_all(&staging); // ignore-ok: probe scratch folder
}
