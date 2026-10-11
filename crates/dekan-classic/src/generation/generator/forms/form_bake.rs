use super::*;
use crate::gear_toggle::bin_error;

impl StandardChampion {
    pub(crate) fn bake_form(
        &self,
        source: &[u8],
        generated: Vec<u8>,
        form: u32,
    ) -> Result<Vec<u8>, ClassicError> {
        let keys = crate::forms::gear_keys(source)?;
        let key = *keys.get(form as usize).ok_or_else(|| {
            ClassicError::Bin(format!(
                "form {form} does not exist; the skin has {} forms",
                keys.len()
            ))
        })?;
        let gear = self.gear_body(source, key)?;
        let mut file = parse_prop_file(&generated).map_err(bin_error)?;
        let submeshes = self.submeshes_of(&file, &gear)?;
        crate::forms::bake_form(
            &mut file,
            &crate::forms::GearForm {
                index: form,
                gear_body: &gear,
                submeshes: &submeshes,
            },
        )?;
        serialize_prop_file(&file).map_err(bin_error)
    }

    pub(crate) fn submeshes_of(
        &self,
        file: &PropFile,
        gear: &[u8],
    ) -> Result<Vec<String>, ClassicError> {
        let mesh_path = |body: &[u8], path: &[&str]| -> Result<Option<String>, ClassicError> {
            let hashes: Vec<u32> = path.iter().map(|p| prop_key_hash(p)).collect();
            Ok(field_value(body, &hashes)
                .map_err(bin_error)?
                .and_then(|v| {
                    v.bytes
                        .get(2..)
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                }))
        };
        let skn = match mesh_path(gear, &["mGearData", "skinMeshProperties", "simpleSkin"])? {
            Some(path) => Some(path),
            None => match file
                .entries
                .iter()
                .find(|e| e.class_hash == SKIN_DATA_CLASS)
            {
                Some(skin) => mesh_path(&skin.body, &["skinMeshProperties", "simpleSkin"])?,
                None => None,
            },
        };
        let Some(skn) = skn else {
            return Ok(Vec::new());
        };
        match self.wad.read(wad_path_hash(&skn.to_ascii_lowercase()))? {
            Some(bytes) => crate::forms::skn_submesh_names(&bytes),
            None => Ok(Vec::new()),
        }
    }
}
