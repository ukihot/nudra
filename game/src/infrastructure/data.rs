//! 定義データ（RON）の読込・Domainへの変換・Catalog（基本設計 5.5, 3.11）。
//! DTOとDomainは分離し、Domainの型にはserdeを付けない。

use std::{
    collections::{BTreeMap, HashMap},
    num::NonZeroU32,
};

use anyhow::{Context, bail};
// Bevy 0.20 の prelude は `ContextExt`（`.context()`）を含み `anyhow::Context`
// と衝突するため、 anyhow で文脈を付けるこのファイルでは prelude を glob で読み込まない。
use bevy::{
    asset::{LoadState, LoadedFolder, RecursiveDependencyLoadState},
    prelude::{
        App, AppExit, Asset, AssetServer, Assets, Commands, Deref, Handle, In, IntoScheduleConfigs,
        IntoSystem, MessageWriter, NextState, OnEnter, Res, ResMut, Resource, TypePath, Update,
        error, in_state, info,
    },
};
use domain::{
    difficulty::{Difficulty, UnknownDifficulty},
    id::{BridgeId, EnemyTypeId, GoalId, MedalId, PusherCharacterId, RailId, StageId},
    math::Vec3,
    positive::{NotPositive, Positive},
    pusher::{MovementPattern, PusherCharacter, PusherStats},
    repository::{PusherCharacterRepository, StageRepository},
    spec::{
        EnemySpec, EvolutionRules, MedalDef, MedalSpec, MedalTier, SpecCatalog, SpecError,
        SpecParts,
    },
    stage::{
        Abyss, Bridge, Catapult, CatapultParts, Geometry, Goal, InitialMedalPlacement, Rail, Shape,
        SpawnEntry, Stage, StageError, StageParts, Wave, WavePlan,
    },
    time::Ticks,
};
use serde::Deserialize;

use super::session::TICK_HZ;
use crate::{
    port::{CharacterRoster, RosterEntry},
    ron_asset,
    state::AppState,
};

// ── DTO ───────────────────────────────────────────────────

#[derive(Asset, TypePath, Deserialize, Debug)]
pub struct StageAsset {
    id: String,
    geometry: GeometryDto,
    catapult: CatapultDto,
    bridges: Vec<BridgeDto>,
    abysses: Vec<AbyssDto>,
    rails: Vec<RailDto>,
    goals: Vec<GoalDto>,
    initial_medals: Vec<GridDto>,
    waves: Vec<WaveDto>,
}

#[derive(Deserialize, Debug)]
enum ShapeDto {
    Rectangle { width: f32, depth: f32 },
    Circle { radius: f32 },
}

#[derive(Deserialize, Debug)]
struct GeometryDto {
    shape: ShapeDto,
    floor_y: f32,
}

#[derive(Deserialize, Debug)]
struct CatapultDto {
    position: [f32; 3],
    base_yaw_deg: f32,
    yaw_range_deg: (f32, f32),
    initial_yaw_deg: f32,
    pitch_deg: f32,
    speed: f32,
    fire_interval_ms: u64,
}

#[derive(Deserialize, Debug)]
struct BridgeDto {
    id: u16,
    entry: [f32; 3],
    exit: [f32; 3],
    width: f32,
}

#[derive(Deserialize, Debug)]
struct AbyssDto {
    min: [f32; 3],
    max: [f32; 3],
}

#[derive(Deserialize, Debug)]
struct RailDto {
    id: u16,
    points: Vec<[f32; 3]>,
}

#[derive(Deserialize, Debug)]
struct GoalDto {
    id: u16,
    position: [f32; 3],
    radius: f32,
    /// この難易度（1〜4）以上で有効。省略時は1（Easy）
    #[serde(default = "easy")]
    available_from: u8,
}

fn easy() -> u8 {
    Difficulty::EASY.level()
}

#[derive(Deserialize, Debug)]
struct GridDto {
    origin: [f32; 3],
    columns: u32,
    rows: u32,
    layers: u32,
    spacing: [f32; 3],
}

#[derive(Deserialize, Debug)]
struct SpawnDto {
    at_ms: u64,
    enemy: String,
    bridge: u16,
    goal: u16,
}

#[derive(Deserialize, Debug)]
struct WaveDto {
    spawns: Vec<SpawnDto>,
    boss: SpawnDto,
}

impl SpawnDto {
    fn to_domain(&self) -> SpawnEntry {
        SpawnEntry {
            at: Ticks::from_millis_ceil(self.at_ms, TICK_HZ),
            enemy: EnemyTypeId::new(&self.enemy),
            bridge: BridgeId(self.bridge),
            goal: GoalId(self.goal),
        }
    }
}

#[derive(Asset, TypePath, Deserialize, Debug)]
pub struct CharacterAsset {
    id: String,
    name: String,
    speed: f32,
    mass: f32,
    push_force: f32,
    push_width: f32,
    pause_at_ends: f32,
}

#[derive(Asset, TypePath, Deserialize, Debug)]
pub struct SpecsAsset {
    medals: Vec<MedalDefDto>,
    enemies: Vec<EnemySpecDto>,
    fusions: Vec<(String, String, String)>,
    evolution: EvolutionDto,
}

#[derive(Deserialize, Debug, Clone, Copy)]
enum MedalTierDto {
    Normal,
    Element,
    Fusion,
}

#[derive(Deserialize, Debug)]
struct MedalDefDto {
    id: String,
    tier: MedalTierDto,
    levels: [MedalSpecDto; 4],
}

#[derive(Deserialize, Debug)]
struct EvolutionDto {
    threshold: NonZeroU32,
    points_per_medal: [u32; 4],
}

#[derive(Deserialize, Debug)]
struct MedalSpecDto {
    mass: f32,
    radius: f32,
    thickness: f32,
    friction: f32,
    restitution: f32,
}

#[derive(Deserialize, Debug)]
struct EnemySpecDto {
    id: String,
    mass: f32,
    radius: f32,
    height: f32,
    max_speed: f32,
    walk_force: f32,
    friction: f32,
}

// ── DTO → Domain ─────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error(transparent)]
    Difficulty(#[from] UnknownDifficulty),
    #[error(transparent)]
    NotPositive(#[from] NotPositive),
    #[error(transparent)]
    Stage(#[from] StageError),
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error("duplicate medal id: {0}")]
    DuplicateMedal(MedalId),
}

fn v(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

fn pos(value: f32) -> Result<Positive, DataError> {
    Ok(Positive::new(value)?)
}

impl StageAsset {
    pub fn to_domain(&self) -> Result<Stage, DataError> {
        let c = &self.catapult;
        let parts = StageParts {
            id: StageId::new(&self.id),
            geometry: Geometry {
                shape: match self.geometry.shape {
                    ShapeDto::Rectangle { width, depth } => {
                        Shape::Rectangle { width: pos(width)?, depth: pos(depth)? }
                    },
                    ShapeDto::Circle { radius } => Shape::Circle { radius: pos(radius)? },
                },
                floor_y: self.geometry.floor_y,
            },
            catapult: Catapult::new(CatapultParts {
                position: v(c.position),
                base_yaw: c.base_yaw_deg.to_radians(),
                yaw_range: (c.yaw_range_deg.0.to_radians(), c.yaw_range_deg.1.to_radians()),
                initial_yaw: c.initial_yaw_deg.to_radians(),
                pitch: c.pitch_deg.to_radians(),
                speed: c.speed,
                fire_interval: Ticks::from_millis_ceil(c.fire_interval_ms, TICK_HZ),
            })?,
            bridges: self
                .bridges
                .iter()
                .map(|b| {
                    Ok(Bridge {
                        id: BridgeId(b.id),
                        entry: v(b.entry),
                        exit: v(b.exit),
                        width: pos(b.width)?,
                    })
                })
                .collect::<Result<_, DataError>>()?,
            abysses: self
                .abysses
                .iter()
                .map(|a| Abyss::new(v(a.min), v(a.max)))
                .collect::<Result<_, _>>()?,
            rails: self
                .rails
                .iter()
                .map(|r| Rail::new(RailId(r.id), r.points.iter().copied().map(v).collect()))
                .collect::<Result<_, _>>()?,
            goals: self
                .goals
                .iter()
                .map(|g| {
                    Ok(Goal {
                        id: GoalId(g.id),
                        position: v(g.position),
                        radius: pos(g.radius)?,
                        available_from: Difficulty::new(g.available_from)?,
                    })
                })
                .collect::<Result<_, DataError>>()?,
            initial_medals: self
                .initial_medals
                .iter()
                .map(|g| {
                    InitialMedalPlacement::grid(
                        v(g.origin),
                        g.columns,
                        g.rows,
                        g.layers,
                        v(g.spacing),
                    )
                })
                .collect(),
            wave_plan: WavePlan::new(
                self.waves
                    .iter()
                    .map(|wave| {
                        Wave::new(
                            wave.spawns.iter().map(SpawnDto::to_domain).collect(),
                            wave.boss.to_domain(),
                        )
                    })
                    .collect(),
            ),
        };
        Ok(Stage::new(parts)?)
    }
}

impl CharacterAsset {
    pub fn to_domain(&self) -> Result<PusherCharacter, DataError> {
        Ok(PusherCharacter {
            id: PusherCharacterId::new(&self.id),
            name: self.name.clone(),
            base_stats: PusherStats {
                speed: pos(self.speed)?,
                mass: pos(self.mass)?,
                push_force: pos(self.push_force)?,
                push_width: pos(self.push_width)?,
            },
            movement: MovementPattern::PingPong { pause_at_ends: self.pause_at_ends },
        })
    }
}

impl SpecsAsset {
    pub fn to_domain(&self) -> Result<SpecCatalog, DataError> {
        let mut medals = BTreeMap::new();
        for def in &self.medals {
            let id = MedalId::new(&def.id);
            let levels = def
                .levels
                .iter()
                .map(|m| {
                    Ok(MedalSpec {
                        mass: pos(m.mass)?,
                        radius: pos(m.radius)?,
                        thickness: pos(m.thickness)?,
                        friction: m.friction,
                        restitution: m.restitution,
                    })
                })
                .collect::<Result<Vec<_>, DataError>>()?;
            let tier = match def.tier {
                MedalTierDto::Normal => MedalTier::Normal,
                MedalTierDto::Element => MedalTier::Element,
                MedalTierDto::Fusion => MedalTier::Fusion,
            };
            let levels = levels.try_into().expect("DTO has exactly four levels");
            if medals
                .insert(id.clone(), MedalDef { tier, levels })
                .is_some()
            {
                return Err(DataError::DuplicateMedal(id));
            }
        }
        let enemies = self
            .enemies
            .iter()
            .map(|e| {
                let spec = EnemySpec {
                    mass: pos(e.mass)?,
                    radius: pos(e.radius)?,
                    height: pos(e.height)?,
                    max_speed: e.max_speed,
                    walk_force: e.walk_force,
                    friction: e.friction,
                };
                Ok((EnemyTypeId::new(&e.id), spec))
            })
            .collect::<Result<HashMap<_, _>, DataError>>()?;
        Ok(SpecCatalog::new(SpecParts {
            medals,
            enemies,
            fusions: self
                .fusions
                .iter()
                .map(|(a, b, result)| (MedalId::new(a), MedalId::new(b), MedalId::new(result)))
                .collect(),
            evolution: EvolutionRules {
                threshold: self.evolution.threshold,
                points_per_medal: self.evolution.points_per_medal,
            },
        })?)
    }
}

// ── Catalog（Repository実装） ─────────────────────────────

#[derive(Resource, Debug, Default)]
pub struct StageCatalog(pub HashMap<StageId, Stage>);

impl StageCatalog {
    /// 【仮決定】ステージ選択は未確定のため、ID順で先頭のステージを使う。
    pub fn default_stage(&self) -> Option<&StageId> {
        self.0.keys().min()
    }
}

impl StageRepository for StageCatalog {
    fn find(&self, id: &StageId) -> Option<Stage> {
        self.0.get(id).cloned()
    }
}

#[derive(Resource, Debug, Default)]
pub struct CharacterCatalog(pub HashMap<PusherCharacterId, PusherCharacter>);

impl PusherCharacterRepository for CharacterCatalog {
    fn find(&self, id: &PusherCharacterId) -> Option<PusherCharacter> {
        self.0.get(id).cloned()
    }
}

#[derive(Resource, Debug, Deref)]
pub struct Specs(pub SpecCatalog);

// ── 読込 ──────────────────────────────────────────────────

const STAGE_DIR: &str = "data/stages";
const CHARACTER_DIR: &str = "data/characters";
const SPECS_FILE: &str = "data/game.specs.ron";

/// ロード中のハンドル。ホットリロード用に読込後も保持する。
#[derive(Resource)]
struct DataHandles {
    stages: Handle<LoadedFolder>,
    characters: Handle<LoadedFolder>,
    specs: Handle<SpecsAsset>,
}

pub(super) fn plugin(app: &mut App) {
    ron_asset::register::<StageAsset>(app, &["stage.ron"]);
    ron_asset::register::<CharacterAsset>(app, &["character.ron"]);
    ron_asset::register::<SpecsAsset>(app, &["specs.ron"]);
    app.add_systems(OnEnter(AppState::Loading), start_loading)
        .add_systems(
            Update,
            finish_loading
                .pipe(exit_on_error)
                .run_if(in_state(AppState::Loading)),
        );
}

fn start_loading(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(DataHandles {
        stages: server.load_folder(STAGE_DIR),
        characters: server.load_folder(CHARACTER_DIR),
        specs: server.load(SPECS_FILE),
    });
}

/// 全データの読込を待ち、Domainへ変換・検証してCatalogを登録する。
/// 1件でも不正なら起動を中止する（基本設計 8.5）。
#[allow(clippy::too_many_arguments)]
fn finish_loading(
    mut commands: Commands,
    handles: Res<DataHandles>,
    server: Res<AssetServer>,
    folders: Res<Assets<LoadedFolder>>,
    stage_assets: Res<Assets<StageAsset>>,
    character_assets: Res<Assets<CharacterAsset>>,
    specs_assets: Res<Assets<SpecsAsset>>,
    mut next: ResMut<NextState<AppState>>,
) -> anyhow::Result<()> {
    let ids = [
        handles.stages.id().untyped(),
        handles.characters.id().untyped(),
        handles.specs.id().untyped(),
    ];
    for id in ids {
        if let LoadState::Failed(err) = server.load_state(id) {
            bail!("failed to load {:?}: {err}", server.get_path(id));
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(id)
        {
            bail!("failed to load a file under {:?}: {err}", server.get_path(id));
        }
    }
    if !ids
        .iter()
        .all(|&id| server.is_loaded_with_dependencies(id))
    {
        return Ok(());
    }

    let specs = specs_assets
        .get(&handles.specs)
        .context("specs asset missing after load")?
        .to_domain()
        .context(SPECS_FILE)?;

    let mut stages = StageCatalog::default();
    for handle in &folder(&folders, &handles.stages)?.handles {
        let path = handle
            .path()
            .map(ToString::to_string)
            .unwrap_or_default();
        let stage = stage_assets
            .get(&handle.clone().typed::<StageAsset>())
            .with_context(|| format!("{path} is not a stage asset"))?
            .to_domain()
            .with_context(|| format!("invalid stage data: {path}"))?;
        specs
            .check_stage(&stage)
            .with_context(|| format!("invalid stage data: {path}"))?;
        stages.0.insert(stage.id().clone(), stage);
    }

    let mut characters = CharacterCatalog::default();
    for handle in &folder(&folders, &handles.characters)?.handles {
        let path = handle
            .path()
            .map(ToString::to_string)
            .unwrap_or_default();
        let character = character_assets
            .get(&handle.clone().typed::<CharacterAsset>())
            .with_context(|| format!("{path} is not a character asset"))?
            .to_domain()
            .with_context(|| format!("invalid character data: {path}"))?;
        characters.0.insert(character.id.clone(), character);
    }

    if stages.0.is_empty() || characters.0.is_empty() {
        bail!("at least one stage and one character are required");
    }

    let mut roster: Vec<_> = characters
        .0
        .values()
        .map(|c| RosterEntry { id: c.id.clone(), name: c.name.clone() })
        .collect();
    roster.sort_by(|a, b| a.id.cmp(&b.id));

    info!(
        stages = stages.0.len(),
        characters = characters.0.len(),
        "definition data loaded"
    );
    commands.insert_resource(CharacterRoster(roster));
    commands.insert_resource(stages);
    commands.insert_resource(characters);
    commands.insert_resource(Specs(specs));
    next.set(AppState::CharacterSelect);
    Ok(())
}

fn folder<'a>(
    folders: &'a Assets<LoadedFolder>,
    handle: &Handle<LoadedFolder>,
) -> anyhow::Result<&'a LoadedFolder> {
    folders.get(handle).context("loaded folder missing")
}

/// 致命的な失敗: 記録して終了する（基本設計 8.5）。
fn exit_on_error(In(result): In<anyhow::Result<()>>, mut exit: MessageWriter<AppExit>) {
    if let Err(e) = result {
        error!("{e:?}");
        exit.write(AppExit::error());
    }
}

#[cfg(test)]
mod tests {
    use domain::evolution::MedalLevel;
    use rstest::rstest;

    use super::*;

    fn specs_asset() -> SpecsAsset {
        ron::from_str(include_str!("../../assets/data/game.specs.ron")).expect("valid RON")
    }

    #[test]
    fn bundled_specs_include_all_sixteen_elements_at_four_levels() {
        let asset = specs_asset();
        let catalog = asset.to_domain().expect("valid specs");
        assert_eq!(catalog.elements().count(), 16);
        for medal in catalog.elements() {
            for level in 1..=4 {
                assert!(
                    catalog
                        .medal(medal, MedalLevel::new(level).expect("level"))
                        .is_some()
                );
            }
        }
    }

    #[test]
    fn all_72_recipes_match_the_supplied_matrix_in_both_directions() {
        let catalog = specs_asset().to_domain().expect("valid catalog");
        let names: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../../locales/ja/medals.json"))
                .expect("medal names");
        let id_for_name = |name: &str| {
            MedalId::new(
                names
                    .iter()
                    .find(|(_, value)| value.as_str() == name)
                    .expect("name has an ID")
                    .0,
            )
        };
        let rows: Vec<Vec<&str>> = include_str!("../../../docs/medal-fusions.csv")
            .lines()
            .map(|line| line.split(',').collect())
            .collect();
        assert_eq!(catalog.medals().count(), 89);
        assert_eq!(catalog.fusions().count(), 72);
        assert_eq!(rows.len(), 17);
        for i in 1..17 {
            assert_eq!(rows[i].len(), 17);
            assert_eq!(rows[i][0], rows[0][i]);
            for j in 1..17 {
                assert_eq!(rows[i][j], rows[j][i], "matrix must be symmetric");
                let expected = match rows[i][j] {
                    "×" | "—" => None,
                    name => Some(id_for_name(name)),
                };
                assert_eq!(
                    catalog.fusion(&id_for_name(rows[i][0]), &id_for_name(rows[0][j])),
                    expected,
                    "{} + {}",
                    rows[i][0],
                    rows[0][j]
                );
            }
        }
    }

    #[rstest]
    #[case("fire", "water", "sauna")]
    #[case("fire", "poison", "spicy_food")]
    #[case("star", "purification", "wish")]
    #[case("blade", "lightning", "chainsaw")]
    fn casual_fusion_recipes_are_symmetric(#[case] a: &str, #[case] b: &str, #[case] result: &str) {
        let catalog = specs_asset().to_domain().expect("valid specs");
        assert_eq!(
            catalog.fusion(&MedalId::new(a), &MedalId::new(b)),
            Some(MedalId::new(result))
        );
        assert_eq!(
            catalog.fusion(&MedalId::new(b), &MedalId::new(a)),
            Some(MedalId::new(result))
        );
    }

    #[test]
    fn duplicate_medal_ids_are_rejected_before_building_the_catalog() {
        let mut asset = specs_asset();
        asset.medals[1].id = asset.medals[0].id.clone();
        assert!(matches!(asset.to_domain(), Err(DataError::DuplicateMedal(_))));
    }

    #[test]
    fn invalid_physics_in_a_later_level_is_rejected() {
        let mut asset = specs_asset();
        asset.medals[0].levels[3].mass = f32::NAN;
        assert!(matches!(asset.to_domain(), Err(DataError::NotPositive(_))));
    }
}
