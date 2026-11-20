//! インゲーム開始（要件16.1、基本設計 4.4）。

use domain::{
    difficulty::Difficulty,
    event::DomainEvent,
    id::{GameSessionId, PusherCharacterId, RailId, StageId},
    math::Vec3,
    progress::{PlayerProgress, ProgressError},
    pusher::{MovementState, PusherCharacter, PusherStats},
    repository::{PusherCharacterRepository, StageRepository},
    session::GameSession,
    spec::SpecCatalog,
    stage::Stage,
};

#[derive(Debug, Clone)]
pub struct StartGameCommand {
    pub session_id: GameSessionId,
    pub stage_id: StageId,
    pub character_id: PusherCharacterId,
    pub difficulty: Difficulty,
    pub seed: u64,
}

/// Pusherの初期配置。
#[derive(Debug, Clone, PartialEq)]
pub struct PusherSetup {
    pub rail: RailId,
    pub motion: MovementState,
    pub position: Vec3,
    pub stats: PusherStats,
}

#[derive(Debug)]
pub struct StartGameOutput {
    pub session: GameSession,
    pub stage: Stage,
    pub character: PusherCharacter,
    pub pusher: PusherSetup,
    pub event: DomainEvent,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StartGameError {
    #[error("stage not found: {0}")]
    StageNotFound(StageId),
    #[error("pusher character not found: {0}")]
    CharacterNotFound(PusherCharacterId),
    #[error(transparent)]
    Progress(#[from] ProgressError),
}

/// 開始の指図の受付検証。実行段階（`start_game`）に失敗し得る条件を残さないよう、受付時に済ませる。
/// ステージ → キャラクター → 難易度の解放、の順に調べ、最初の失敗を返す。
pub fn check_start(
    stages: &impl StageRepository,
    characters: &impl PusherCharacterRepository,
    progress: &PlayerProgress,
    stage: &StageId,
    character: &PusherCharacterId,
    difficulty: Difficulty,
) -> Result<(), StartGameError> {
    if stages.find(stage).is_none() {
        return Err(StartGameError::StageNotFound(stage.clone()));
    }
    if characters.find(character).is_none() {
        return Err(StartGameError::CharacterNotFound(character.clone()));
    }
    Ok(progress.ensure_unlocked(stage, difficulty)?)
}

pub fn start_game(
    stages: &impl StageRepository,
    characters: &impl PusherCharacterRepository,
    specs: &SpecCatalog,
    command: StartGameCommand,
) -> Result<StartGameOutput, StartGameError> {
    let stage = stages
        .find(&command.stage_id)
        .ok_or(StartGameError::StageNotFound(command.stage_id))?;
    let character = characters
        .find(&command.character_id)
        .ok_or(StartGameError::CharacterNotFound(command.character_id))?;
    let (session, event) = GameSession::start(
        command.session_id,
        &stage,
        &character,
        command.difficulty,
        specs.evolution(),
        command.seed,
    );

    let rail = stage
        .rails()
        .first()
        .expect("Stage invariant S3: at least one rail");
    let motion = MovementState::start(rail);
    let pusher = PusherSetup {
        rail: rail.id(),
        motion,
        position: motion.target(rail),
        // Pusherは成長しない（基本設計 3.12）
        stats: character.base_stats,
    };
    tracing::info!(session_id = %session.id(), stage_id = %stage.id(), character_id = %character.id, "game started");
    Ok(StartGameOutput { session, stage, character, pusher, event })
}

#[cfg(test)]
mod tests {
    use domain::session::GameStatus;
    use mockall::predicate::eq;
    use rstest::rstest;

    use super::*;
    use crate::test_support::{MockCharacters, MockStages, character, specs, stage};

    fn command() -> StartGameCommand {
        StartGameCommand {
            session_id: GameSessionId(42),
            stage_id: StageId::new("stage_01"),
            character_id: PusherCharacterId::new("mop_rabbit"),
            difficulty: Difficulty::HARDEST,
            seed: 42,
        }
    }

    fn stages_returning(stage: Option<Stage>) -> MockStages {
        let mut mock = MockStages::new();
        mock.expect_find()
            .with(eq(StageId::new("stage_01")))
            .times(1)
            .return_const(stage);
        mock
    }

    fn characters_returning(character: Option<PusherCharacter>) -> MockCharacters {
        let mut mock = MockCharacters::new();
        mock.expect_find()
            .with(eq(PusherCharacterId::new("mop_rabbit")))
            .times(1)
            .return_const(character);
        mock
    }

    #[rstest]
    fn starts_session_and_places_pusher_on_first_rail(stage: Stage, character: PusherCharacter) {
        let out = start_game(
            &stages_returning(Some(stage)),
            &characters_returning(Some(character.clone())),
            &specs(),
            command(),
        )
        .expect("started");

        assert_eq!(out.session.id(), GameSessionId(42));
        assert_eq!(out.session.status(), GameStatus::Playing);
        assert_eq!(
            out.session.difficulty(),
            Difficulty::HARDEST,
            "difficulty comes from the command"
        );
        assert_eq!(out.stage.id(), &StageId::new("stage_01"));
        assert_eq!(out.character, character);
        assert_eq!(
            out.event,
            DomainEvent::GameStarted {
                session: GameSessionId(42),
                stage: StageId::new("stage_01"),
                character: PusherCharacterId::new("mop_rabbit"),
            }
        );
        assert_eq!(out.pusher.rail, RailId(7));
        assert_eq!(out.pusher.position, Vec3::new(-2.0, 0.5, 0.0));
        assert_eq!(out.pusher.stats, character.base_stats);
    }

    #[rstest]
    fn unknown_stage_is_reported_without_loading_character() {
        let mut characters = MockCharacters::new();
        characters.expect_find().never();
        assert_eq!(
            start_game(&stages_returning(None), &characters, &specs(), command()).err(),
            Some(StartGameError::StageNotFound(StageId::new("stage_01")))
        );
    }

    #[rstest]
    fn unknown_character_is_reported(stage: Stage) {
        assert_eq!(
            start_game(
                &stages_returning(Some(stage)),
                &characters_returning(None),
                &specs(),
                command()
            )
            .err(),
            Some(StartGameError::CharacterNotFound(PusherCharacterId::new(
                "mop_rabbit"
            )))
        );
    }

    // ── 受付検証 ──

    fn check(
        stages: &MockStages,
        characters: &MockCharacters,
        difficulty: Difficulty,
    ) -> Result<(), StartGameError> {
        check_start(
            stages,
            characters,
            &PlayerProgress::default(),
            &StageId::new("stage_01"),
            &PusherCharacterId::new("mop_rabbit"),
            difficulty,
        )
    }

    #[rstest]
    fn start_on_an_unlocked_difficulty_is_accepted(stage: Stage, character: PusherCharacter) {
        assert_eq!(
            check(
                &stages_returning(Some(stage)),
                &characters_returning(Some(character)),
                Difficulty::EASY
            ),
            Ok(())
        );
    }

    #[rstest]
    fn start_on_a_locked_difficulty_is_rejected(stage: Stage, character: PusherCharacter) {
        assert_eq!(
            check(
                &stages_returning(Some(stage)),
                &characters_returning(Some(character)),
                Difficulty::HARDEST
            ),
            Err(StartGameError::Progress(ProgressError::LockedDifficulty {
                stage: StageId::new("stage_01"),
                requested: Difficulty::HARDEST,
            }))
        );
    }

    #[rstest]
    fn start_with_an_unknown_stage_is_rejected_without_loading_character() {
        let mut characters = MockCharacters::new();
        characters.expect_find().never();
        assert_eq!(
            check(&stages_returning(None), &characters, Difficulty::EASY),
            Err(StartGameError::StageNotFound(StageId::new("stage_01")))
        );
    }

    #[rstest]
    fn start_with_an_unknown_character_is_rejected(stage: Stage) {
        assert_eq!(
            check(
                &stages_returning(Some(stage)),
                &characters_returning(None),
                Difficulty::EASY
            ),
            Err(StartGameError::CharacterNotFound(PusherCharacterId::new(
                "mop_rabbit"
            )))
        );
    }
}
