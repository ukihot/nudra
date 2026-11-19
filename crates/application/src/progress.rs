//! 進行記録の読込・記録・保存（オフラインファースト、基本設計 5.7）。
//! 保存の失敗はゲーム進行を止めない。記録はメモリ上で保持し、次の保存で再試行する。

use domain::{
    progress::{PlayerProgress, ProgressError, RecordOutcome},
    repository::ProgressRepository,
    session::{GameSession, GameStatus},
};

/// 進行中のセッションは記録しない（操作の拒否）。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RecordResultError {
    #[error(transparent)]
    Progress(#[from] ProgressError),
    #[error("session is still in progress")]
    NotFinished,
}

/// 保存先の技術的な失敗。分岐不要なため型で細分化しない（基本設計 8.3）。
#[derive(Debug, thiserror::Error)]
pub enum ProgressStoreError {
    #[error(transparent)]
    Infrastructure(#[from] anyhow::Error),
}

/// 保存が無ければ空の記録から始める。
pub fn load_progress(repo: &impl ProgressRepository) -> Result<PlayerProgress, ProgressStoreError> {
    Ok(repo
        .load()
        .map_err(anyhow::Error::new)?
        .unwrap_or_default())
}

pub fn save_progress(
    repo: &mut impl ProgressRepository,
    progress: &PlayerProgress,
) -> Result<(), ProgressStoreError> {
    Ok(repo.save(progress).map_err(anyhow::Error::new)?)
}

/// 終了したセッションの結果を記録する。
pub fn record_result(
    progress: &mut PlayerProgress,
    session: &GameSession,
) -> Result<RecordOutcome, RecordResultError> {
    if session.status() == GameStatus::Playing {
        return Err(RecordResultError::NotFinished);
    }
    Ok(progress.record_session_result(
        session.stage_id(),
        session.defeated(),
        session.difficulty(),
        session.status() == GameStatus::Won,
    )?)
}

#[cfg(test)]
mod tests {

    use domain::id::{EnemyId, GoalId, PusherCharacterId, StageId};
    use mockall::predicate::eq;
    use rstest::rstest;

    use super::*;
    use crate::test_support::{session, stage};

    #[derive(Debug, thiserror::Error)]
    #[error("disk full")]
    struct DiskFull;

    mockall::mock! {
        Store {}
        impl ProgressRepository for Store {
            type Error = DiskFull;
            fn load(&self) -> Result<Option<PlayerProgress>, DiskFull>;
            fn save(&mut self, progress: &PlayerProgress) -> Result<(), DiskFull>;
        }
    }

    fn saved_progress() -> PlayerProgress {
        PlayerProgress::restore(
            Some(PusherCharacterId::new("mop_rabbit")),
            3,
            [(StageId::new("stage_01"), 3, 7)],
        )
        .expect("valid")
    }

    #[rstest]
    fn load_without_save_starts_empty() {
        let mut store = MockStore::new();
        store.expect_load().times(1).returning(|| Ok(None));
        assert_eq!(load_progress(&store).expect("loaded"), PlayerProgress::default());
    }

    #[rstest]
    fn load_returns_saved_progress() {
        let mut store = MockStore::new();
        store
            .expect_load()
            .times(1)
            .returning(|| Ok(Some(saved_progress())));
        assert_eq!(load_progress(&store).expect("loaded"), saved_progress());
    }

    #[rstest]
    fn load_failure_keeps_the_cause() {
        let mut store = MockStore::new();
        store
            .expect_load()
            .times(1)
            .returning(|| Err(DiskFull));
        let err = load_progress(&store).expect_err("load fails");
        assert_eq!(err.to_string(), "disk full");
    }

    #[rstest]
    fn save_passes_progress_to_store() {
        let mut store = MockStore::new();
        store
            .expect_save()
            .with(eq(saved_progress()))
            .times(1)
            .returning(|_| Ok(()));
        save_progress(&mut store, &saved_progress()).expect("saved");
    }

    #[rstest]
    fn save_failure_keeps_the_cause() {
        let mut store = MockStore::new();
        store
            .expect_save()
            .times(1)
            .returning(|_| Err(DiskFull));
        let err = save_progress(&mut store, &saved_progress()).expect_err("save fails");
        assert_eq!(err.to_string(), "disk full");
    }

    #[rstest]
    fn finished_session_is_recorded(mut session: GameSession, stage: domain::stage::Stage) {
        session.advance(&stage, domain::time::Tick(60));
        session.enemy_fell(EnemyId(1));
        session.enemy_reached_goal(EnemyId(2), GoalId(1));
        let mut progress = PlayerProgress::default();

        let outcome = record_result(&mut progress, &session).expect("finished");

        assert_eq!(outcome, RecordOutcome { best_defeated: 1, new_record: true });
        let record = progress
            .record(&StageId::new("stage_01"))
            .expect("recorded");
        assert_eq!((record.plays(), record.best_defeated()), (1, 1));
    }

    #[rstest]
    fn session_in_progress_is_not_recorded(session: GameSession) {
        let mut progress = PlayerProgress::default();
        assert_eq!(
            record_result(&mut progress, &session),
            Err(RecordResultError::NotFinished)
        );
        assert_eq!(progress, PlayerProgress::default());
    }

    #[rstest]
    fn play_limit_is_returned_without_modifying_progress(
        mut session: GameSession,
        stage: domain::stage::Stage,
    ) {
        session.advance(&stage, domain::time::Tick(60));
        session.enemy_reached_goal(EnemyId(1), GoalId(1));
        let mut progress =
            PlayerProgress::restore(None, u32::MAX, [(stage.id().clone(), u32::MAX, 7)])
                .expect("valid progress");
        let before = progress.clone();
        assert_eq!(
            record_result(&mut progress, &session),
            Err(RecordResultError::Progress(ProgressError::PlayLimitReached))
        );
        assert_eq!(progress, before);
    }
}
