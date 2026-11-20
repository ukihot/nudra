//! 能力の実行時の安全予算（基本設計 3.18）。
//!
//! 1能力に必要な枠 [`Need`]
//! をまとめて予約し、どれか1つでも足りなければ何も消費せずに全体を棄却する。
//! 上限値は性能保証ではない（実機の計測で調整する）。上限に達したときの意味だけを先に固定する。

/// 上限【仮決定の初期値】。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    /// 能力の発動（再生先も1回） / tick
    pub fires: u32,
    /// `OtherFell` の候補の検査 / tick
    pub other_fell_checks: u32,
    /// 能力によるメダルの生成・分割 / tick
    pub spawns: u32,
    /// 生存しているメダル / セッション（初期配置・射出・生成・分割、予約済みを含む）
    pub live_medals: u32,
    /// 同時に存在する場 / セッション
    pub zones: u32,
    /// 対象への効果の適用（対象数 × 効果数） / tick
    pub effects: u32,
    /// 次tickへ回す出来事（生成直後の `Entered` など） / セッション
    pub queue: u32,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            fires: 256,
            other_fell_checks: 4_096,
            spawns: 64,
            live_medals: 3_000,
            zones: 128,
            effects: 8_192,
            queue: 1_024,
        }
    }
}

/// 1能力の適用に必要な枠（発動1回は常に含む）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Need {
    pub spawns: u32,
    pub zones: u32,
    pub effects: u32,
    pub queued: u32,
}

/// 足りなかった枠。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetExceeded {
    #[error("too many abilities fired this tick")]
    Fires,
    #[error("too many medals spawned this tick")]
    Spawns,
    #[error("the board is full of medals")]
    LiveMedals,
    #[error("too many zones on the board")]
    Zones,
    #[error("too many effects applied this tick")]
    Effects,
    #[error("too many queued happenings")]
    Queue,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TickUsage {
    fires: u32,
    checks: u32,
    spawns: u32,
    effects: u32,
}

/// 予算の使用状況。tickごとの枠と、セッションを通した枠を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    limits: BudgetLimits,
    tick: TickUsage,
    live: u32,
    zones: u32,
    queued: u32,
}

impl Budget {
    /// 初期配置だけで生存数の上限を超えるステージは開始しない。
    pub fn new(limits: BudgetLimits, initial_medals: u32) -> Result<Self, BudgetExceeded> {
        if initial_medals > limits.live_medals {
            return Err(BudgetExceeded::LiveMedals);
        }
        Ok(Self {
            limits,
            tick: TickUsage::default(),
            live: initial_medals,
            zones: 0,
            queued: 0,
        })
    }

    pub fn live_medals(&self) -> u32 {
        self.live
    }

    /// 能力1つ分をまとめて予約する。足りなければ何も変えずに `Err`。
    pub fn reserve(&mut self, need: Need) -> Result<(), BudgetExceeded> {
        let (l, t) = (self.limits, self.tick);
        let checks = [
            (fits(t.fires, 1, l.fires), BudgetExceeded::Fires),
            (fits(t.spawns, need.spawns, l.spawns), BudgetExceeded::Spawns),
            (
                fits(self.live, need.spawns, l.live_medals),
                BudgetExceeded::LiveMedals,
            ),
            (fits(self.zones, need.zones, l.zones), BudgetExceeded::Zones),
            (fits(t.effects, need.effects, l.effects), BudgetExceeded::Effects),
            (fits(self.queued, need.queued, l.queue), BudgetExceeded::Queue),
        ];
        if let Some((_, exceeded)) = checks.iter().find(|(ok, _)| !ok) {
            return Err(*exceeded);
        }
        self.tick.fires += 1;
        self.tick.spawns += need.spawns;
        self.live += need.spawns;
        self.zones += need.zones;
        self.tick.effects += need.effects;
        self.queued += need.queued;
        Ok(())
    }

    /// プレイヤーの射出1枚分（盤面が満杯なら射出しない）。
    pub fn reserve_launch(&mut self) -> Result<(), BudgetExceeded> {
        if !fits(self.live, 1, self.limits.live_medals) {
            return Err(BudgetExceeded::LiveMedals);
        }
        self.live += 1;
        Ok(())
    }

    /// `OtherFell` の候補を1件検査してよいか。よければ1消費する（不成立の候補も数える）。
    pub fn check_other_fell(&mut self) -> bool {
        let ok = self.tick.checks < self.limits.other_fell_checks;
        if ok {
            self.tick.checks += 1;
        }
        ok
    }

    /// 場の再適用（各tickの場1つ分の対象効果）。足りなければそのtickの適用全体を省略する。
    pub fn reserve_effects(&mut self, effects: u32) -> Result<(), BudgetExceeded> {
        if !fits(self.tick.effects, effects, self.limits.effects) {
            return Err(BudgetExceeded::Effects);
        }
        self.tick.effects += effects;
        Ok(())
    }

    /// メダルが盤面から退場した（復活は退場ではない）。
    pub fn medal_left(&mut self) {
        self.live = self.live.saturating_sub(1);
    }

    pub fn zone_ended(&mut self) {
        self.zones = self.zones.saturating_sub(1);
    }

    /// 待ち行列から出来事を取り出した。
    pub fn dequeued(&mut self, n: u32) {
        self.queued = self.queued.saturating_sub(n);
    }

    /// 次のtickへ（tickごとの枠を戻す）。
    pub fn next_tick(&mut self) {
        self.tick = TickUsage::default();
    }
}

/// `used + add <= limit`（オーバーフローは超過として扱う）。
fn fits(used: u32, add: u32, limit: u32) -> bool {
    used.checked_add(add).is_some_and(|v| v <= limit)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn limits() -> BudgetLimits {
        BudgetLimits {
            fires: 2,
            other_fell_checks: 3,
            spawns: 3,
            live_medals: 10,
            zones: 2,
            effects: 5,
            queue: 4,
        }
    }

    fn budget(initial: u32) -> Budget {
        Budget::new(limits(), initial).expect("within the limit")
    }

    #[rstest]
    #[case::below(9, true)]
    #[case::exactly(10, true)]
    #[case::over(11, false)]
    fn the_initial_placement_must_fit(#[case] initial: u32, #[case] ok: bool) {
        assert_eq!(Budget::new(limits(), initial).is_ok(), ok);
    }

    #[rstest]
    #[case::fires(Need::default(), 2, None)]
    #[case::spawns_exactly(Need { spawns: 3, ..Need::default() }, 1, None)]
    #[case::spawns_over(Need { spawns: 4, ..Need::default() }, 0, Some(BudgetExceeded::Spawns))]
    #[case::zones_over(Need { zones: 3, ..Need::default() }, 0, Some(BudgetExceeded::Zones))]
    #[case::effects_over(Need { effects: 6, ..Need::default() }, 0, Some(BudgetExceeded::Effects))]
    #[case::queue_over(Need { queued: 5, ..Need::default() }, 0, Some(BudgetExceeded::Queue))]
    fn a_need_is_reserved_up_to_the_limits(
        #[case] need: Need,
        #[case] times: usize,
        #[case] first_error: Option<BudgetExceeded>,
    ) {
        let mut b = budget(0);
        for _ in 0..times {
            assert_eq!(b.reserve(need), Ok(()));
        }
        let expected = first_error.unwrap_or(if times == 2 {
            BudgetExceeded::Fires
        } else {
            BudgetExceeded::Spawns
        });
        assert_eq!(b.reserve(need), Err(expected));
    }

    #[rstest]
    fn a_failed_reservation_consumes_nothing() {
        let mut b = budget(0);
        b.reserve(Need { spawns: 2, zones: 1, effects: 3, queued: 2 })
            .expect("fits");
        let before = b.clone();
        // 生成だけが足りない（3枚まで、あと1枚）
        assert_eq!(
            b.reserve(Need { spawns: 2, zones: 1, effects: 1, queued: 1 }),
            Err(BudgetExceeded::Spawns)
        );
        assert_eq!(b, before);
    }

    #[rstest]
    fn spawns_count_against_the_live_medals_too() {
        let mut b = budget(9);
        assert_eq!(
            b.reserve(Need { spawns: 2, ..Need::default() }),
            Err(BudgetExceeded::LiveMedals)
        );
        assert_eq!(b.reserve(Need { spawns: 1, ..Need::default() }), Ok(()));
        assert_eq!(b.live_medals(), 10);
        assert_eq!(
            b.reserve_launch(),
            Err(BudgetExceeded::LiveMedals),
            "the board is full"
        );
        b.medal_left();
        assert_eq!(b.reserve_launch(), Ok(()));
    }

    #[rstest]
    fn tick_limits_reset_but_session_limits_do_not() {
        let mut b = budget(0);
        b.reserve(Need { spawns: 3, zones: 2, effects: 5, queued: 4 })
            .expect("fits");
        b.reserve(Need::default()).expect("second fire");
        b.next_tick();
        assert_eq!(
            b.reserve(Need { spawns: 3, effects: 5, ..Need::default() }),
            Ok(()),
            "per-tick limits reset"
        );
        assert_eq!(
            b.reserve(Need { zones: 1, ..Need::default() }),
            Err(BudgetExceeded::Zones)
        );
        b.zone_ended();
        b.next_tick();
        assert_eq!(b.reserve(Need { zones: 1, ..Need::default() }), Ok(()));
        assert_eq!(
            b.reserve(Need { queued: 1, ..Need::default() }),
            Err(BudgetExceeded::Queue)
        );
        b.dequeued(4);
        assert_eq!(
            b.reserve(Need { queued: 4, ..Need::default() }),
            Ok(()),
            "the queue was freed"
        );
        assert_eq!(
            b.reserve(Need::default()),
            Err(BudgetExceeded::Fires),
            "fires are per tick"
        );
    }

    #[rstest]
    fn other_fell_checks_stop_at_the_limit() {
        let mut b = budget(0);
        assert_eq!((0..5).filter(|_| b.check_other_fell()).count(), 3);
        b.next_tick();
        assert!(b.check_other_fell());
    }

    #[rstest]
    fn zone_effects_share_the_effect_limit() {
        let mut b = budget(0);
        assert_eq!(b.reserve_effects(5), Ok(()));
        assert_eq!(b.reserve_effects(1), Err(BudgetExceeded::Effects));
        assert_eq!(
            b.reserve(Need { effects: 1, ..Need::default() }),
            Err(BudgetExceeded::Effects)
        );
    }

    #[rstest]
    fn counters_never_underflow() {
        let mut b = budget(0);
        b.medal_left();
        b.zone_ended();
        b.dequeued(10);
        assert_eq!(b, budget(0));
    }
}
