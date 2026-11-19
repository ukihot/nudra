//! プレイヤーの進行記録（オフラインファーストのセーブ対象、基本設計 5.7）。
//! インゲームの成長は持ち越さない（要件9.1）。ここに残すのはセッションをまたぐ記録だけ。

use std::collections::BTreeMap;

use crate::id::{PusherCharacterId, StageId};

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
        Ok(Self { last_character, games_played, records: map })
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

    /// 1回のゲーム結果を記録する。回数は飽和加算（上限で止まる）。
    pub fn record_result(&mut self, stage: &StageId, defeated: u32) -> RecordOutcome {
        // ponytail: 総プレイ回数が u32::MAX（約43億回）に達したら記録を止める。
        // 回数の整合（ステージ合計 ≦ 総数）を保ち、保存データが復元で拒否されないようにするため。
        if self.games_played == u32::MAX {
            let best_defeated = self
                .record(stage)
                .map_or(0, StageRecord::best_defeated);
            return RecordOutcome { best_defeated, new_record: false };
        }
        self.games_played += 1;
        let record = self
            .records
            .entry(stage.clone())
            .or_insert(StageRecord { plays: 0, best_defeated: 0 });
        record.plays += 1;
        let new_record = record.plays == 1 || defeated > record.best_defeated;
        record.best_defeated = record.best_defeated.max(defeated);
        RecordOutcome { best_defeated: record.best_defeated, new_record }
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
        let outcome = progress.record_result(&stage("s1"), 5);
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
        progress.record_result(&stage("s1"), first);
        assert_eq!(progress.record_result(&stage("s1"), second), expected);
        assert_eq!(progress.record(&stage("s1")).map(StageRecord::plays), Some(2));
        assert_eq!(progress.games_played(), 2);
    }

    #[rstest]
    fn counters_saturate_instead_of_overflowing() {
        let mut progress =
            PlayerProgress::restore(None, u32::MAX, [(stage("s1"), u32::MAX, 1)]).expect("valid");
        progress.record_result(&stage("s1"), 2);
        assert_eq!(progress.games_played(), u32::MAX);
        assert_eq!(progress.record(&stage("s1")).map(StageRecord::plays), Some(u32::MAX));
    }

    #[rstest]
    fn saturated_progress_still_restores() {
        let mut progress =
            PlayerProgress::restore(None, u32::MAX, [(stage("s1"), u32::MAX, 1)]).expect("valid");
        progress.record_result(&stage("s2"), 9);
        let restored = PlayerProgress::restore(
            None,
            progress.games_played(),
            progress
                .records()
                .map(|(id, r)| (id.clone(), r.plays(), r.best_defeated())),
        );
        assert_eq!(restored, Ok(progress));
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
        fn recorded_progress_always_restores(results in prop::collection::vec((0u8..4, any::<u32>()), 0..40)) {
            let mut progress = PlayerProgress::default();
            for (stage_no, defeated) in results {
                progress.record_result(&stage(&format!("s{stage_no}")), defeated);
            }
            let restored = PlayerProgress::restore(
                progress.last_character().cloned(),
                progress.games_played(),
                progress.records().map(|(id, r)| (id.clone(), r.plays(), r.best_defeated())),
            );
            prop_assert_eq!(restored, Ok(progress));
        }
    }
}
