use super::*;

pub(crate) struct FormAudio {
    pub(crate) graph: Vec<u8>,
    pub(crate) bank_path: String,
    pub(crate) bank: Vec<u8>,
    pub(crate) skin0: Vec<u8>,
}

impl StandardChampion {
    fn sound_switches(
        &self,
        source: &[u8],
        forms: usize,
    ) -> Option<(String, crate::form_sound::FormSounds)> {
        let paths = match crate::form_sound::bank_paths(source) {
            Ok(paths) => paths,
            Err(e) => {
                warn!(alias = %self.alias, error = %e, "The skin's sound banks could not be listed");
                return None;
            }
        };
        for path in paths {
            let added = self
                .wad
                .read(wad_path_hash(&path.to_ascii_lowercase()))
                .map_err(ClassicError::from)
                .and_then(|bank| match bank {
                    Some(bank) => crate::form_sound::add_form_switches(&bank, forms),
                    None => Ok(None),
                });
            match added {
                Ok(Some(sounds)) => return Some((path, sounds)),
                Ok(None) => {}
                Err(e) => {
                    warn!(alias = %self.alias, bank = %path, error = %e, "Sound bank left as the game ships it")
                }
            }
        }
        None
    }

    pub(crate) fn form_audio(
        &self,
        source: &[u8],
        graph: &[u8],
        graph_key: u32,
        forms: usize,
        skin0: &[u8],
    ) -> Option<FormAudio> {
        let (bank_path, sounds) = self.sound_switches(source, forms)?;
        let applied =
            crate::form_sound::fire_on_entry(graph, graph_key, &sounds.events).and_then(|graph| {
                let skin0 = crate::form_sound::list_events(skin0, &bank_path, &sounds.events)?;
                Ok((graph, skin0))
            });
        match applied {
            Ok((graph, skin0)) => {
                debug!(alias = %self.alias, bank = %bank_path, events = sounds.events.len(), "Each form sets the skin's gear sound switches");
                Some(FormAudio {
                    graph,
                    bank_path,
                    bank: sounds.bank,
                    skin0,
                })
            }
            Err(e) => {
                warn!(alias = %self.alias, error = %e, "Form sounds not added; every form keeps the first form's sounds");
                None
            }
        }
    }
}
