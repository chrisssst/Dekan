use super::*;

#[test]
fn test_animation_lines_are_grouped_with_their_first_time_and_count() {
    let log = "000010.5| WARN| Missing animation clip Spell1 for Viego\n\
               000011.0| WARN| Missing animation clip Spell1 for Viego\n\
               000012.0| ALWAYS| Connected to server\n\
               no separator but a submesh line\n";
    let lines = animation_lines(log);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        lines.get("WARN| Missing animation clip Spell1 for Viego"),
        Some(&("000010.5".to_owned(), 2))
    );
    assert!(lines.contains_key("no separator but a submesh line"));
}

#[test]
fn test_the_number_of_distinct_lines_is_capped_and_tracing_runs_with_logs_on() {
    let log: String = (0..MAX_DISTINCT + 20)
        .map(|n| format!("{n}| mesh line {n}\n"))
        .collect();
    assert_eq!(animation_lines(&log).len(), MAX_DISTINCT);

    trace(&log);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .finish();
    tracing::subscriber::with_default(subscriber, || trace(&log));
}
