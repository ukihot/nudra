//! 指図台帳（基本設計 6.4）。
//!
//! 提出された指図はすべて記帳し、`受付待ち → 完了 / 却下 / 期限切れ` のいずれかの終端へ必ず移す。
//! 受付（`take_*`）で取り出した指図は、取り出した側が実行して完了か却下を通知する。
//! 台帳に残っているものは、フレームの終わり（`end_frame`）に状態違い・期限切れを判定する。

use bevy::prelude::Resource;

use crate::port::{RejectReason, Request, RequestId, RequestOutcome, RequestStatus};

/// 受付待ちのまま待てるフレーム数（調整ノブ）。
pub const TTL_FRAMES: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Navigate,
    SetLanguage,
    Start,
    Evolve,
    SelectMedal,
    Menu,
}

impl RequestKind {
    pub fn of(request: &Request) -> Self {
        match request {
            Request::Navigate(_) => Self::Navigate,
            Request::SetLanguage(_) => Self::SetLanguage,
            Request::Start { .. } => Self::Start,
            Request::Evolve(_) => Self::Evolve,
            Request::SelectMedal(_) => Self::SelectMedal,
            Request::Menu => Self::Menu,
        }
    }
}

#[derive(Debug)]
struct Entry {
    id: RequestId,
    request: Request,
    age: u32,
}

#[derive(Resource, Debug, Default)]
pub struct RequestLedger {
    next_id: u64,
    pending: Vec<Entry>,
}

pub fn outcome(id: RequestId, request: Request, status: RequestStatus) -> RequestOutcome {
    RequestOutcome { id, request, status }
}

impl RequestLedger {
    /// 記帳する。ID は提出順に振る。
    pub fn submit(&mut self, request: Request) -> RequestId {
        let id = RequestId(self.next_id);
        self.next_id += 1;
        self.pending.push(Entry { id, request, age: 0 });
        id
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 指定の種類の受付待ちをすべて受け付ける（提出順）。
    pub fn take_all(&mut self, kind: RequestKind) -> Vec<(RequestId, Request)> {
        let (taken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|entry| RequestKind::of(&entry.request) == kind);
        self.pending = kept;
        taken
            .into_iter()
            .map(|entry| (entry.id, entry.request))
            .collect()
    }

    /// 指定の種類の最初の1件を受け付ける。同じ受付で重複した残りは `Duplicate` で終端にする。
    pub fn take_first(
        &mut self,
        kind: RequestKind,
    ) -> (Option<(RequestId, Request)>, Vec<RequestOutcome>) {
        let mut taken = self.take_all(kind).into_iter();
        let first = taken.next();
        let duplicates = taken
            .map(|(id, request)| {
                outcome(id, request, RequestStatus::Rejected(RejectReason::Duplicate))
            })
            .collect();
        (first, duplicates)
    }

    /// フレームの終わり。今の状態で受け付けない種類は `WrongState`、
    /// 期限（[`TTL_FRAMES`]）を過ぎたものは `Expired` で終端にする。
    pub fn end_frame(&mut self, accepts: impl Fn(RequestKind) -> bool) -> Vec<RequestOutcome> {
        let mut closed = Vec::new();
        for mut entry in std::mem::take(&mut self.pending) {
            if !accepts(RequestKind::of(&entry.request)) {
                let status = RequestStatus::Rejected(RejectReason::WrongState);
                closed.push(outcome(entry.id, entry.request, status));
                continue;
            }
            entry.age += 1;
            if entry.age >= TTL_FRAMES {
                closed.push(outcome(entry.id, entry.request, RequestStatus::Expired));
            } else {
                self.pending.push(entry);
            }
        }
        closed
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use domain::id::PusherCharacterId;
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;

    fn start(name: &str) -> Request {
        Request::Start {
            character: PusherCharacterId::new(name),
            stage: domain::id::StageId::new("stage_01"),
            difficulty: domain::difficulty::Difficulty::EASY,
        }
    }

    #[rstest]
    fn submitted_requests_get_increasing_ids() {
        let mut ledger = RequestLedger::default();
        assert_eq!(ledger.submit(start("a")), RequestId(0));
        assert_eq!(ledger.submit(Request::Menu), RequestId(1));
        assert_eq!(ledger.pending_len(), 2);
    }

    #[rstest]
    fn take_first_accepts_one_and_rejects_duplicates() {
        let mut ledger = RequestLedger::default();
        ledger.submit(start("a"));
        ledger.submit(Request::Menu);
        ledger.submit(start("b"));

        let (accepted, rejected) = ledger.take_first(RequestKind::Start);

        assert_eq!(accepted, Some((RequestId(0), start("a"))));
        assert_eq!(
            rejected,
            [outcome(
                RequestId(2),
                start("b"),
                RequestStatus::Rejected(RejectReason::Duplicate)
            )]
        );
        assert_eq!(ledger.pending_len(), 1, "other kinds stay pending");
    }

    #[rstest]
    fn take_first_with_nothing_pending() {
        let mut ledger = RequestLedger::default();
        ledger.submit(Request::Menu);
        assert_eq!(ledger.take_first(RequestKind::Start), (None, vec![]));
        assert_eq!(ledger.pending_len(), 1);
    }

    #[rstest]
    fn take_all_keeps_submission_order() {
        let mut ledger = RequestLedger::default();
        ledger.submit(start("a"));
        ledger.submit(Request::Menu);
        ledger.submit(start("b"));
        let ids: Vec<_> = ledger
            .take_all(RequestKind::Start)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, [RequestId(0), RequestId(2)]);
        assert_eq!(ledger.pending_len(), 1);
    }

    #[rstest]
    fn requests_not_accepted_in_this_state_are_rejected() {
        let mut ledger = RequestLedger::default();
        ledger.submit(start("a"));
        ledger.submit(Request::Menu);
        let outcomes = ledger.end_frame(|kind| kind == RequestKind::Menu);
        assert_eq!(
            outcomes,
            [outcome(
                RequestId(0),
                start("a"),
                RequestStatus::Rejected(RejectReason::WrongState)
            )]
        );
        assert_eq!(ledger.pending_len(), 1, "menu still waits for its turn");
    }

    #[rstest]
    fn waiting_requests_expire_after_ttl() {
        let mut ledger = RequestLedger::default();
        ledger.submit(Request::Menu);
        for _ in 0..TTL_FRAMES - 1 {
            assert!(ledger.end_frame(|_| true).is_empty());
        }
        assert_eq!(
            ledger.end_frame(|_| true),
            [outcome(RequestId(0), Request::Menu, RequestStatus::Expired)]
        );
        assert_eq!(ledger.pending_len(), 0);
    }

    #[derive(Debug, Clone)]
    enum Op {
        Submit(u8),
        TakeFirst(u8),
        TakeAll(u8),
        EndFrame(u8),
    }

    fn kind(n: u8) -> RequestKind {
        [
            RequestKind::Start,
            RequestKind::Evolve,
            RequestKind::Menu,
            RequestKind::SelectMedal,
            RequestKind::Navigate,
            RequestKind::SetLanguage,
        ][usize::from(n % 6)]
    }

    fn request(n: u8) -> Request {
        match kind(n) {
            RequestKind::Start => start("a"),
            RequestKind::Evolve => Request::Evolve(domain::evolution::Evolution::LevelUp(
                domain::id::MedalId::normal(),
            )),
            RequestKind::SelectMedal => Request::SelectMedal(domain::id::MedalId::normal()),
            RequestKind::Menu => Request::Menu,
            RequestKind::Navigate => Request::Navigate(crate::port::OutgameScreen::Title),
            RequestKind::SetLanguage => Request::SetLanguage(domain::progress::Language::Japanese),
        }
    }

    proptest! {
        /// どんな操作列でも、提出した指図はちょうど1回だけ終端に至る（受付 or 終端通知）。
        #[test]
        fn every_request_reaches_exactly_one_end(ops in prop::collection::vec(
            prop_oneof![
                any::<u8>().prop_map(Op::Submit),
                any::<u8>().prop_map(Op::TakeFirst),
                any::<u8>().prop_map(Op::TakeAll),
                any::<u8>().prop_map(Op::EndFrame),
            ],
            0..200,
        )) {
            let mut ledger = RequestLedger::default();
            let mut submitted = 0u64;
            let mut ends: BTreeMap<RequestId, usize> = BTreeMap::new();
            let mut finish = |id: RequestId| *ends.entry(id).or_default() += 1;
            for op in ops {
                match op {
                    Op::Submit(n) => { ledger.submit(request(n)); submitted += 1; },
                    Op::TakeFirst(n) => {
                        let (accepted, rejected) = ledger.take_first(kind(n));
                        accepted.into_iter().for_each(|(id, _)| finish(id));
                        rejected.into_iter().for_each(|o| finish(o.id));
                    },
                    Op::TakeAll(n) => ledger.take_all(kind(n)).into_iter().for_each(|(id, _)| finish(id)),
                    Op::EndFrame(mask) => ledger
                        .end_frame(|k| mask & (1 << (k as u8)) != 0)
                        .into_iter()
                        .for_each(|o| finish(o.id)),
                }
            }
            // どの状態でも受け付けない状態でフレームを終えると、残りはすべて終端に至る
            ledger.end_frame(|_| false).into_iter().for_each(|o| finish(o.id));
            prop_assert_eq!(ledger.pending_len(), 0);
            prop_assert_eq!(ends.len() as u64, submitted);
            prop_assert!(ends.values().all(|&n| n == 1));
        }
    }
}
