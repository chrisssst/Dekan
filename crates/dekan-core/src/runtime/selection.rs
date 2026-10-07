pub type SkinId = u32;

pub type ChampionId = u32;

pub type ChromaId = u32;

#[must_use]
pub fn is_base_skin(skin_id: SkinId, champion_id: ChampionId) -> bool {
    skin_id == 0 || skin_id == champion_id * 1000
}
