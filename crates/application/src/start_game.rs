//! インゲーム開始（要件16.1、基本設計 4.4）。

use domain::{
    event::DomainEvent,
    id::{GameSessionId, PusherCharacterId, RailId, StageId},
    math::Vec3,
    pusher::{MovementState, PusherCharacter, PusherStats},
    repository::{PusherCharacterRepository, StageRepository},
    session::GameSession,
    stage::Stage,
};

#[derive(Debug, Clone)]
pub struct StartGameCommand {
    pub session_id: GameSessionId,
    pub stage_id: StageId,
    pub character_id: PusherCharacterId,
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
}

pub fn start_game(
    stages: &impl StageRepository,
    characters: &impl PusherCharacterRepository,
    command: StartGameCommand,
) -> Result<StartGameOutput, StartGameError> {
    let stage = stages
        .find(&command.stage_id)
        .ok_or(StartGameError::StageNotFound(command.stage_id))?;
    let character = characters
        .find(&command.character_id)
        .ok_or(StartGameError::CharacterNotFound(command.character_id))?;
    let (session, event) = GameSession::start(command.session_id, &stage, &character);

    let rail = stage
        .rails()
        .first()
        .expect("Stage invariant S3: at least one rail");
    let motion = MovementState::start(rail);
    let pusher = PusherSetup {
        rail: rail.id(),
        motion,
        position: motion.target(rail),
        stats: session.pusher_stats(&character),
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
    use crate::test_support::{MockCharacters, MockStages, character, stage};

    fn command() -> StartGameCommand {
        StartGameCommand {
            session_id: GameSessionId(42),
            stage_id: StageId::new("stage_01"),
            character_id: PusherCharacterId::new("mop_rabbit"),
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
            command(),
        )
        .expect("started");

        assert_eq!(out.session.id(), GameSessionId(42));
        assert_eq!(out.session.status(), GameStatus::Playing);
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
            start_game(&stages_returning(None), &characters, command()).err(),
            Some(StartGameError::StageNotFound(StageId::new("stage_01")))
        );
    }

    #[rstest]
    fn unknown_character_is_reported(stage: Stage) {
        assert_eq!(
            start_game(&stages_returning(Some(stage)), &characters_returning(None), command())
                .err(),
            Some(StartGameError::CharacterNotFound(PusherCharacterId::new(
                "mop_rabbit"
            )))
        );
    }
}
