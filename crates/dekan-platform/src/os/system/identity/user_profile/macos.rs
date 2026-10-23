use super::Resolution;

pub(super) fn resolve() -> Resolution {
    match std::env::home_dir() {
        Some(home) if home.is_absolute() => {
            let path = home.join("Library").join("Application Support");
            match std::fs::create_dir_all(&path) {
                Ok(()) => Resolution::SameUser { path },
                Err(e) => Resolution::Unresolved {
                    reason: format!("{} could not be created: {e}", path.display()),
                },
            }
        }
        _ => Resolution::Unresolved {
            reason: "the user has no home folder".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn the_data_folder_is_application_support_under_home() {
        let Resolution::SameUser { path } = resolve() else {
            panic!("unresolved");
        };
        assert!(path.ends_with(PathBuf::from("Library/Application Support")));
    }
}
