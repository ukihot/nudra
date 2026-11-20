//! メダルの能力（基本設計 3.14〜3.16）。ECS で起きた出来事から能力を発動し、ECS への指示を返す。
//!
//! 範囲内の対象の検索と物理は ECS が行い、ここは印の変化と発動のルールだけを Domain
//! に問い合わせる。 印の効き目 `modifiers` は変わったときだけ返す（ECS
//! は物理へ1回だけ反映し、毎フレーム同期しない）。

pub use domain::firing::{
    AbilityOrder, AbilityPlan, EffectOrder, Occasion, PlanRejection, StalePlan,
};
use domain::{
    ability::Effect,
    event::DomainEvent,
    firing,
    mark::{Conditions, Modifiers},
    session::GameSession,
    spec::SpecCatalog,
    time::Tick,
    unit::MedalUnit,
};

use crate::budget::{Budget, BudgetExceeded, Need};

/// 印が変わった結果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkOutput {
    /// 印が変わったときの新しい効き目（変わらなければ `None`）
    pub modifiers: Option<Modifiers>,
    /// 反応で増えた、状態以外の効果（対象の位置で ECS が実行する）
    pub reactions: Vec<Effect>,
}

/// 期限切れの結果。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ExpireOutput {
    pub modifiers: Option<Modifiers>,
    /// 持ち主が消える（能力による消滅。通常の報酬は出さない、基本設計 3.18）
    pub vanish: bool,
}

/// 能力1つを計画する（セッションも個体も変更しない）。
pub fn plan_ability(
    session: &GameSession,
    specs: &SpecCatalog,
    unit: &MedalUnit,
    index: usize,
    occasion: Occasion,
) -> Result<AbilityPlan, PlanRejection> {
    firing::plan(session, specs, unit, index, occasion)
}

/// 計画の適用に必要な枠。`targets` は指示ごとの範囲内の対象数（`plan.orders` と同じ順）。
/// 対象効果は「対象数 × 対象に当てる効果の数」。生成した子の `Entered` は次tickの待ち行列を使う。
pub fn need_of(plan: &AbilityPlan, targets: &[u32]) -> Need {
    let mut need = Need {
        spawns: plan.spawns(),
        zones: plan.zones(),
        ..Need::default()
    };
    for (order, &targets) in plan.orders.iter().zip(targets) {
        let per_target = order.states.len()
            + order
                .actions
                .iter()
                .filter(|a| {
                    matches!(
                        a,
                        EffectOrder::Transform { .. }
                            | EffectOrder::Apply(
                                Effect::Push { .. } | Effect::Remove | Effect::Split
                            )
                    )
                })
                .count();
        let per_target = u32::try_from(per_target).unwrap_or(u32::MAX);
        need.effects = need
            .effects
            .saturating_add(targets.saturating_mul(per_target));
        need.spawns = need
            .spawns
            .saturating_add(targets.saturating_mul(order.splits()));
    }
    // 生成・分割した子の `Entered` は次tickの待ち行列で処理する（基本設計 3.18）
    need.queued = need.spawns;
    need
}

/// 予約と鮮度の確認が済んだ計画。ECSへ適用してから [`commit_ability`] で確定する。複製できない。
#[derive(Debug)]
pub struct ApprovedAbility(AbilityPlan);

impl ApprovedAbility {
    /// ECSへ適用する指示（生成・場・対象への効果）。
    pub fn plan(&self) -> &AbilityPlan {
        &self.0
    }
}

/// 承認できなかった理由。どちらの場合も何も変更していない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApproveError {
    #[error(transparent)]
    Budget(#[from] BudgetExceeded),
    #[error(transparent)]
    Stale(#[from] StalePlan),
}

/// ECSへ適用する直前に呼ぶ:
/// 計画が今の状態から作られたかを確かめ、予算を予約する（能力全体の受理）。
pub fn approve_ability(
    session: &GameSession,
    unit: &MedalUnit,
    plan: AbilityPlan,
    budget: &mut Budget,
    need: Need,
) -> Result<ApprovedAbility, ApproveError> {
    if !plan.is_fresh(session, unit) {
        return Err(StalePlan.into());
    }
    budget.reserve(need)?;
    Ok(ApprovedAbility(plan))
}

/// ECSへの適用を終えてから確定する（承認から確定までの間に別の確定を挟まない。基本設計 3.16）。
/// ここで `Err` になるのは呼び出し側の約束違反で、セッションを安全に中断する（基本設計 8.5）。
pub fn commit_ability(
    session: &mut GameSession,
    specs: &SpecCatalog,
    unit: &mut MedalUnit,
    approved: ApprovedAbility,
) -> Result<Vec<DomainEvent>, StalePlan> {
    let events = firing::commit(session, specs, unit, approved.0)?;
    if !events.is_empty() {
        tracing::debug!(medal = %unit.medal(), ?events, "ability committed");
    }
    Ok(events)
}

/// 能力の範囲内の対象1体に、印・浄化（[`AbilityOrder::states`]）を当てる。
pub fn hit_target(
    specs: &SpecCatalog,
    target: &mut Conditions,
    states: &[Effect],
    now: Tick,
) -> MarkOutput {
    let book = specs.abilities();
    let hit = book.hit(target, states, now);
    MarkOutput {
        modifiers: hit.changed.then(|| target.modifiers(|m| book.mark(m))),
        reactions: hit.effects,
    }
}

/// 場の中にいる対象1体（毎 tick）。場の余韻の長さの印を付ける。場でない効果は何もしない。
pub fn stand_in_zone(
    specs: &SpecCatalog,
    target: &mut Conditions,
    zone: &Effect,
    now: Tick,
) -> MarkOutput {
    zone.zone_mark()
        .map_or_else(MarkOutput::default, |mark| {
            hit_target(specs, target, std::slice::from_ref(&mark), now)
        })
}

/// 期限の来た印を外す。
pub fn expire_marks(specs: &SpecCatalog, target: &mut Conditions, now: Tick) -> ExpireOutput {
    let book = specs.abilities();
    let expiry = target.expire(now, |m| book.mark(m));
    ExpireOutput {
        modifiers: expiry
            .changed
            .then(|| target.modifiers(|m| book.mark(m))),
        vanish: expiry.vanish,
    }
}

#[cfg(test)]
mod tests {
    use domain::{
        ability::{Direction, Happening, Partner},
        id::MarkId,
        time::Ticks,
    };
    use rstest::rstest;

    use super::*;
    use crate::{
        budget::BudgetLimits,
        test_support::{ability_specs, fire, lv, p, session},
    };

    fn mark(id: &str, duration: Option<u64>) -> Effect {
        Effect::Mark {
            mark: MarkId::new(id),
            duration: duration.map(Ticks),
        }
    }

    fn collided() -> Occasion {
        Occasion {
            happening: Happening::Collided(Partner::Enemy),
            partner: true,
        }
    }

    fn budget() -> Budget {
        Budget::new(BudgetLimits::default(), 0).expect("empty board")
    }

    #[rstest]
    fn a_collision_plans_the_medal_ability(session: GameSession) {
        let specs = ability_specs();
        let unit = MedalUnit::new(fire(), lv(1));
        let planned = plan_ability(&session, &specs, &unit, 0, collided()).expect("planned");
        assert_eq!(
            planned.orders[0].actions,
            [EffectOrder::Apply(Effect::Push {
                dir: Direction::Outward,
                strength: p(2.0)
            })]
        );
        assert_eq!(planned.orders[0].states, [mark("wet", Some(60))]);
    }

    /// 対象2体に「濡らす＋押す」= 4効果。生成も場もない。
    #[rstest]
    fn the_need_counts_targets_times_effects(session: GameSession) {
        let specs = ability_specs();
        let planned = plan_ability(&session, &specs, &MedalUnit::new(fire(), lv(1)), 0, collided())
            .expect("planned");
        assert_eq!(
            need_of(&planned, &[2]),
            Need { spawns: 0, zones: 0, effects: 4, queued: 0 }
        );
        assert_eq!(need_of(&planned, &[0]), Need::default());
    }

    /// 分割は新しいメダルを生むので、対象数だけ生成（と次tickの出来事）に数える。
    #[rstest]
    fn splits_count_as_spawns_per_target(session: GameSession) {
        use domain::{
            ability::{AbilityDef, Area, CollideWith, Target, Trigger},
            spec::abilities::MedalAbilities,
        };
        let mut parts = crate::test_support::spec_parts();
        parts.abilities.medals = std::collections::BTreeMap::from([(
            fire(),
            MedalAbilities {
                base: vec![AbilityDef {
                    on: Trigger::Collide(CollideWith::Any),
                    target: Target::Medals,
                    area: Area::Radius(p(1.0)),
                    effects: vec![Effect::Split],
                    uses: None,
                }],
                awaken: vec![],
            },
        )]);
        let specs = domain::spec::SpecCatalog::new(parts).expect("valid");
        let planned = plan_ability(&session, &specs, &MedalUnit::new(fire(), lv(1)), 0, collided())
            .expect("planned");
        assert_eq!(
            need_of(&planned, &[3]),
            Need { spawns: 3, zones: 0, effects: 3, queued: 3 }
        );
    }

    #[rstest]
    fn committing_reserves_the_budget(session: GameSession) {
        let specs = ability_specs();
        let mut session = session;
        let mut unit = MedalUnit::new(fire(), lv(1));
        let planned = plan_ability(&session, &specs, &unit, 0, collided()).expect("planned");
        let mut b = budget();
        let need = Need { effects: 8_000, ..Need::default() };
        let approved = approve_ability(&session, &unit, planned, &mut b, need).expect("approved");
        // ここでECSへ適用する
        assert_eq!(commit_ability(&mut session, &specs, &mut unit, approved), Ok(vec![]));
        assert_eq!(session.last_ability().map(|a| a.index), Some(0), "committed");
        assert_eq!(
            b.reserve(Need { effects: 193, ..Need::default() }),
            Err(BudgetExceeded::Effects)
        );
    }

    #[rstest]
    fn a_rejected_budget_changes_nothing(session: GameSession) {
        let specs = ability_specs();
        let unit = MedalUnit::new(fire(), lv(1));
        let planned = plan_ability(&session, &specs, &unit, 0, collided()).expect("planned");
        let mut b = budget();
        let before = b.clone();
        let too_much = Need { spawns: 65, ..Need::default() };
        assert_eq!(
            approve_ability(&session, &unit, planned, &mut b, too_much).err(),
            Some(ApproveError::Budget(BudgetExceeded::Spawns))
        );
        assert_eq!(b, before);
    }

    #[rstest]
    fn a_stale_plan_is_not_approved_and_reserves_nothing(session: GameSession) {
        let specs = ability_specs();
        let mut session = session;
        let mut unit = MedalUnit::new(fire(), lv(1));
        let first = plan_ability(&session, &specs, &unit, 0, collided()).expect("planned");
        let second = plan_ability(&session, &specs, &unit, 0, collided()).expect("planned");
        let mut b = budget();
        let approved =
            approve_ability(&session, &unit, first, &mut b, Need::default()).expect("approved");
        commit_ability(&mut session, &specs, &mut unit, approved).expect("committed");
        let before = b.clone();
        assert_eq!(
            approve_ability(&session, &unit, second, &mut b, Need::default()).err(),
            Some(ApproveError::Stale(StalePlan))
        );
        assert_eq!(b, before, "nothing reserved for a stale plan");
    }
    #[rstest]
    fn hitting_a_target_returns_its_new_modifiers_once() {
        let specs = ability_specs();
        let mut target = Conditions::default();
        let out = hit_target(&specs, &mut target, &[mark("wet", Some(60))], Tick(0));
        assert_eq!(out.modifiers.and_then(|m| m.friction), Some(0.05));
        assert!(out.reactions.is_empty());
        assert_eq!(hit_target(&specs, &mut target, &[], Tick(1)), MarkOutput::default());
    }

    #[rstest]
    fn a_reaction_returns_its_effects() {
        let specs = ability_specs();
        let mut target = Conditions::default();
        hit_target(&specs, &mut target, &[mark("wet", None)], Tick(0));
        let out = hit_target(&specs, &mut target, &[mark("shock", Some(10))], Tick(1));
        assert_eq!(out.reactions, [Effect::Push { dir: Direction::Up, strength: p(3.0) }]);
        assert!(out.modifiers.is_some());
    }

    #[rstest]
    fn standing_in_a_zone_marks_for_its_linger() {
        let specs = ability_specs();
        let mut target = Conditions::default();
        let zone = Effect::Zone {
            mark: MarkId::new("wet"),
            radius: p(2.0),
            duration: Ticks(300),
            linger: Ticks(30),
        };
        assert!(
            stand_in_zone(&specs, &mut target, &zone, Tick(100))
                .modifiers
                .is_some()
        );
        assert_eq!(target.marks()[0].until, Some(Tick(130)));
        assert_eq!(
            stand_in_zone(&specs, &mut target, &Effect::Remove, Tick(101)),
            MarkOutput::default()
        );
    }

    #[rstest]
    fn expiring_marks_report_new_modifiers_and_vanishing() {
        let specs = ability_specs();
        let mut target = Conditions::default();
        hit_target(
            &specs,
            &mut target,
            &[mark("wet", Some(5)), mark("burn", Some(10))],
            Tick(0),
        );
        assert_eq!(expire_marks(&specs, &mut target, Tick(4)), ExpireOutput::default());
        let wet_gone = expire_marks(&specs, &mut target, Tick(5));
        assert_eq!(
            (wet_gone.modifiers.map(|m| m.friction), wet_gone.vanish),
            (Some(None), false)
        );
        assert!(expire_marks(&specs, &mut target, Tick(10)).vanish, "burnt away");
    }
}
