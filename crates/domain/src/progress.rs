//! プレイヤーの進行記録（オフラインファーストのセーブ対象、基本設計 5.7）。
//! インゲームの成長は持ち越さない（要件9.1）。ここに残すのはセッションをまたぐ記録だけ。

use std::collections::BTreeMap;

use crate::{
    difficulty::Difficulty,
    id::{PusherCharacterId, StageId},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Japanese,
    English,
}

/// ステージごとの記録。`plays >= 1` の記録しか存在しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageRecord {
    plays: u32,
    best_defeated: u32,
}

impl StageRecord {
    pub fn plays(self) -> u32 {
        self.plays
    }

    pub fn best_defeated(self) -> u32 {
        self.best_defeated
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProgressError {
    #[error("the maximum number of recorded games has been reached")]
    PlayLimitReached,
    #[error("difficulty {requested:?} is locked for stage {stage}")]
    LockedDifficulty { stage: StageId, requested: Difficulty },
    #[error("clear record for stage {0} is duplicated or has no play record")]
    InvalidClear(StageId),
    #[error("record of stage {0} has zero plays")]
    EmptyRecord(StageId),
    #[error("record of stage {0} appears twice")]
    DuplicateStage(StageId),
    #[error("stage plays ({stage_plays}) exceed games played ({games_played})")]
    PlaysExceedGames { stage_plays: u64, games_played: u32 },
}

/// 記録した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordOutcome {
    pub best_defeated: u32,
    pub new_record: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerProgress {
    last_character: Option<PusherCharacterId>,
    games_played: u32,
    /// ID順に保持する（保存内容を決定的にするため）
    records: BTreeMap<StageId, StageRecord>,
    cleared: BTreeMap<StageId, Difficulty>,
    language: Language,
}

impl PlayerProgress {
    /// 保存データからの復元。不正な値を含む記録は受け付けない（読込時の検証）。
    pub fn restore(
        last_character: Option<PusherCharacterId>,
        games_played: u32,
        records: impl IntoIterator<Item = (StageId, u32, u32)>,
    ) -> Result<Self, ProgressError> {
        let mut map = BTreeMap::new();
        for (stage, plays, best_defeated) in records {
            if plays == 0 {
                return Err(ProgressError::EmptyRecord(stage));
            }
            if map.contains_key(&stage) {
                return Err(ProgressError::DuplicateStage(stage));
            }
            map.insert(stage, StageRecord { plays, best_defeated });
        }
        let stage_plays: u64 = map.values().map(|r| u64::from(r.plays)).sum();
        if stage_plays > u64::from(games_played) {
            return Err(ProgressError::PlaysExceedGames { stage_plays, games_played });
        }
        Ok(Self {
            last_character,
            games_played,
            records: map,
            cleared: BTreeMap::new(),
            language: Language::default(),
        })
    }

    pub fn unlocked_difficulty(&self, stage: &StageId) -> Difficulty {
        let level = self
            .cleared
            .get(stage)
            .map_or(1, |d| (d.level() + 1).min(Difficulty::LEVELS));
        Difficulty::new(level).expect("unlocked level is within 1..=4")
    }

    /// この難易度がステージで解放済みか。開始の受付と結果の記録の両方がこれを使う。
    pub fn ensure_unlocked(
        &self,
        stage: &StageId,
        difficulty: Difficulty,
    ) -> Result<(), ProgressError> {
        if difficulty > self.unlocked_difficulty(stage) {
            return Err(ProgressError::LockedDifficulty {
                stage: stage.clone(),
                requested: difficulty,
            });
        }
        Ok(())
    }

    /// 1回のゲーム結果を記録する唯一の入口。検証失敗時は状態を一切変更しない。
    pub fn record_session_result(
        &mut self,
        stage: &StageId,
        defeated: u32,
        difficulty: Difficulty,
        won: bool,
    ) -> Result<RecordOutcome, ProgressError> {
        self.ensure_unlocked(stage, difficulty)?;
        if self.games_played == u32::MAX {
            return Err(ProgressError::PlayLimitReached);
        }

        // 復元時の不変条件「ステージ回数の合計 <= 総回数」と上の検証により、両方加算できる。
        self.games_played += 1;
        let record = self
            .records
            .entry(stage.clone())
            .or_insert(StageRecord { plays: 0, best_defeated: 0 });
        record.plays += 1;
        let new_record = record.plays == 1 || defeated > record.best_defeated;
        record.best_defeated = record.best_defeated.max(defeated);
        let outcome = RecordOutcome { best_defeated: record.best_defeated, new_record };
        if won {
            self.cleared
                .entry(stage.clone())
                .and_modify(|d| *d = (*d).max(difficulty))
                .or_insert(difficulty);
        }
        Ok(outcome)
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    pub fn clears(&self) -> impl Iterator<Item = (&StageId, Difficulty)> {
        self.cleared.iter().map(|(id, d)| (id, *d))
    }

    /// 新版セーブの追加項目を検証する。旧版のプレイ記録をクリアとは推測しない。
    pub fn restore_preferences(
        mut self,
        language: Language,
        clears: impl IntoIterator<Item = (StageId, Difficulty)>,
    ) -> Result<Self, ProgressError> {
        let mut cleared = BTreeMap::new();
        for (stage, difficulty) in clears {
            if !self.records.contains_key(&stage)
                || cleared.insert(stage.clone(), difficulty).is_some()
            {
                return Err(ProgressError::InvalidClear(stage));
            }
        }
        self.language = language;
        self.cleared = cleared;
        Ok(self)
    }

    pub fn last_character(&self) -> Option<&PusherCharacterId> {
        self.last_character.as_ref()
    }

    pub fn games_played(&self) -> u32 {
        self.games_played
    }

    pub fn record(&self, stage: &StageId) -> Option<StageRecord> {
        self.records.get(stage).copied()
    }

    /// ID順の記録。
    pub fn records(&self) -> impl Iterator<Item = (&StageId, StageRecord)> {
        self.records.iter().map(|(id, record)| (id, *record))
    }

    pub fn select_character(&mut self, character: PusherCharacterId) {
        self.last_character = Some(character);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    fn stage(id: &str) -> StageId {
        StageId::new(id)
    }

    #[test]
    fn wins_unlock_only_the_next_difficulty_on_the_same_stage() {
        let mut progress = PlayerProgress::default();
        let s = stage("s1");
        assert_eq!(progress.unlocked_difficulty(&s), Difficulty::EASY);
        progress
            .record_session_result(&s, 5, Difficulty::EASY, false)
            .expect("loss");
        assert_eq!(progress.unlocked_difficulty(&s), Difficulty::EASY);
        for level in 1..=4 {
            let d = Difficulty::new(level).expect("difficulty");
            progress
                .record_session_result(&s, 5, d, true)
                .expect("win");
            assert_eq!(progress.unlocked_difficulty(&s).level(), (level + 1).min(4));
        }
        assert_eq!(progress.unlocked_difficulty(&stage("other")), Difficulty::EASY);
        progress
            .record_session_result(&s, 5, Difficulty::EASY, true)
            .expect("replay");
        assert_eq!(progress.unlocked_difficulty(&s), Difficulty::HARDEST);
    }

    #[rstest]
    #[case::easy_is_always_open(&[], 1, true)]
    #[case::second_is_locked_at_first(&[], 2, false)]
    #[case::loss_does_not_unlock(&[(1, false)], 2, false)]
    #[case::win_unlocks_the_next(&[(1, true)], 2, true)]
    #[case::win_unlocks_only_the_next(&[(1, true)], 3, false)]
    fn difficulties_unlock_one_by_one(
        #[case] results: &[(u8, bool)],
        #[case] requested: u8,
        #[case] unlocked: bool,
    ) {
        let mut progress = PlayerProgress::default();
        let d = |level| Difficulty::new(level).expect("difficulty");
        for &(level, won) in results {
            progress
                .record_session_result(&stage("s1"), 1, d(level), won)
                .expect("recorded");
        }
        let expected = if unlocked {
            Ok(())
        } else {
            Err(ProgressError::LockedDifficulty { stage: stage("s1"), requested: d(requested) })
        };
        assert_eq!(progress.ensure_unlocked(&stage("s1"), d(requested)), expected);
    }

    #[test]
    fn locked_results_are_rejected_without_changes() {
        let mut progress = PlayerProgress::default();
        let before = progress.clone();
        assert!(
            progress
                .record_session_result(&stage("s1"), 5, Difficulty::HARDEST, true)
                .is_err()
        );
        assert_eq!(progress, before);
    }

    #[test]
    fn preferences_restore_validates_clear_references_and_duplicates() {
        let progress = PlayerProgress::restore(None, 1, [(stage("s1"), 1, 3)]).expect("valid");
        assert!(
            progress
                .clone()
                .restore_preferences(Language::English, [(stage("missing"), Difficulty::EASY)])
                .is_err()
        );
        assert!(
            progress
                .clone()
                .restore_preferences(
                    Language::English,
                    [(stage("s1"), Difficulty::EASY), (stage("s1"), Difficulty::EASY)]
                )
                .is_err()
        );
        let restored = progress
            .restore_preferences(Language::English, [(stage("s1"), Difficulty::EASY)])
            .expect("valid");
        assert_eq!(restored.language(), Language::English);
        assert_eq!(restored.unlocked_difficulty(&stage("s1")).level(), 2);
    }

    #[rstest]
    fn new_progress_is_empty() {
        let progress = PlayerProgress::default();
        assert_eq!(progress.games_played(), 0);
        assert_eq!(progress.last_character(), None);
        assert_eq!(progress.records().count(), 0);
    }

    #[rstest]
    fn first_result_creates_record_and_is_a_new_record() {
        let mut progress = PlayerProgress::default();
        let outcome = progress
            .record_session_result(&stage("s1"), 5, Difficulty::EASY, false)
            .expect("recorded");
        assert_eq!(outcome, RecordOutcome { best_defeated: 5, new_record: true });
        assert_eq!(progress.games_played(), 1);
        let record = progress.record(&stage("s1")).expect("recorded");
        assert_eq!((record.plays(), record.best_defeated()), (1, 5));
    }

    #[rstest]
    #[case::better(3, 7, RecordOutcome { best_defeated: 7, new_record: true })]
    #[case::equal(3, 3, RecordOutcome { best_defeated: 3, new_record: false })]
    #[case::worse(3, 1, RecordOutcome { best_defeated: 3, new_record: false })]
    fn best_is_kept(#[case] first: u32, #[case] second: u32, #[case] expected: RecordOutcome) {
        let mut progress = PlayerProgress::default();
        progress
            .record_session_result(&stage("s1"), first, Difficulty::EASY, false)
            .expect("recorded");
        assert_eq!(
            progress.record_session_result(&stage("s1"), second, Difficulty::EASY, false),
            Ok(expected)
        );
        assert_eq!(progress.record(&stage("s1")).map(StageRecord::plays), Some(2));
        assert_eq!(progress.games_played(), 2);
    }

    #[rstest]
    #[case::existing_stage_loss("s1", false)]
    #[case::existing_stage_win("s1", true)]
    #[case::new_stage_loss("s2", false)]
    #[case::new_stage_win("s2", true)]
    fn play_limit_rejects_the_entire_result_without_changes(#[case] id: &str, #[case] won: bool) {
        let mut progress = PlayerProgress::restore(
            Some(PusherCharacterId::new("bear")),
            u32::MAX,
            [(stage("s1"), u32::MAX, 1)],
        )
        .expect("valid")
        .restore_preferences(Language::English, [])
        .expect("valid preferences");
        let before = progress.clone();
        assert_eq!(
            progress.record_session_result(&stage(id), 9, Difficulty::EASY, won),
            Err(ProgressError::PlayLimitReached)
        );
        assert_eq!(
            progress, before,
            "counts, best score, clears and preferences must all stay unchanged"
        );
    }

    #[test]
    fn last_available_play_records_every_field_and_the_next_play_changes_nothing() {
        let mut progress =
            PlayerProgress::restore(None, u32::MAX - 1, [(stage("s1"), u32::MAX - 1, 1)])
                .expect("valid");
        assert_eq!(
            progress.record_session_result(&stage("s1"), 9, Difficulty::EASY, true),
            Ok(RecordOutcome { best_defeated: 9, new_record: true })
        );
        assert_eq!(progress.games_played(), u32::MAX);
        assert_eq!(progress.record(&stage("s1")).map(StageRecord::plays), Some(u32::MAX));
        assert_eq!(progress.unlocked_difficulty(&stage("s1")).level(), 2);
        let before = progress.clone();
        assert_eq!(
            progress.record_session_result(
                &stage("s1"),
                10,
                Difficulty::new(2).expect("difficulty"),
                true
            ),
            Err(ProgressError::PlayLimitReached)
        );
        assert_eq!(progress, before);
    }

    #[rstest]
    fn selected_character_is_remembered() {
        let mut progress = PlayerProgress::default();
        progress.select_character(PusherCharacterId::new("bear"));
        assert_eq!(progress.last_character(), Some(&PusherCharacterId::new("bear")));
    }

    #[rstest]
    fn restore_accepts_consistent_data() {
        let progress = PlayerProgress::restore(
            Some(PusherCharacterId::new("rabbit")),
            5,
            [(stage("b"), 2, 9), (stage("a"), 3, 4)],
        )
        .expect("valid");
        assert_eq!(progress.games_played(), 5);
        let ids: Vec<_> = progress.records().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a", "b"], "records are ordered by id");
    }

    #[rstest]
    #[case::zero_plays(1, vec![("a", 0, 0)], ProgressError::EmptyRecord(StageId::new("a")))]
    #[case::duplicate(4, vec![("a", 1, 0), ("a", 1, 0)], ProgressError::DuplicateStage(StageId::new("a")))]
    #[case::plays_exceed_games(
        2,
        vec![("a", 2, 0), ("b", 1, 0)],
        ProgressError::PlaysExceedGames { stage_plays: 3, games_played: 2 }
    )]
    fn restore_rejects_inconsistent_data(
        #[case] games_played: u32,
        #[case] records: Vec<(&str, u32, u32)>,
        #[case] expected: ProgressError,
    ) {
        let records = records
            .into_iter()
            .map(|(id, plays, best)| (stage(id), plays, best));
        assert_eq!(PlayerProgress::restore(None, games_played, records), Err(expected));
    }

    proptest! {
        /// 記録をどう重ねても、復元の検証を通る状態のままである。
        #[test]
        fn recorded_progress_always_restores(results in prop::collection::vec((0u8..4, any::<u32>(), any::<bool>()), 0..40)) {
            let mut progress = PlayerProgress::default();
            for (stage_no, defeated, won) in results {
                let stage = stage(&format!("s{stage_no}"));
                let difficulty = progress.unlocked_difficulty(&stage);
                progress.record_session_result(&stage, defeated, difficulty, won).expect("unlocked result");
            }
            let restored = PlayerProgress::restore(
                progress.last_character().cloned(),
                progress.games_played(),
                progress.records().map(|(id, r)| (id.clone(), r.plays(), r.best_defeated())),
            ).and_then(|restored| restored.restore_preferences(progress.language(), progress.clears().map(|(id, d)| (id.clone(), d))));
            prop_assert_eq!(restored, Ok(progress));
        }
    }
}
