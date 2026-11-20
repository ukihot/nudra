//! メダルの能力（基本設計 3.14〜3.16）。ECS で起きた出来事から能力を発動し、ECS への指示を返す。
//!
//! 範囲内の対象の検索と物理は ECS が行い、ここは印の変化と発動のルールだけを Domain
//! に問い合わせる。 印の効き目 `modifiers` は変わったときだけ返す（ECS
//! は物理へ1回だけ反映し、毎フレーム同期しない）。

pub use domain::firing::{AbilityOrder, EffectOrder, Firing};
use domain::{
    ability::{Effect, Happening},
    firing,
    mark::{Conditions, Modifiers},
    session::GameSession,
    spec::SpecCatalog,
    time::Tick,
    unit::MedalUnit,
};

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
    /// 持ち主が消える（消えたメダルは落下と同じに数える）
    pub vanish: bool,
}

/// メダル個体に出来事が起きた。発動した能力を ECS への指示として返す。
pub fn fire_abilities(
    session: &mut GameSession,
    specs: &SpecCatalog,
    unit: &mut MedalUnit,
    happening: Happening,
) -> Firing {
    let out = firing::fire(session, specs, unit, happening);
    if !out.orders.is_empty() {
        tracing::debug!(medal = %unit.medal(), ?happening, orders = out.orders.len(), "abilities fired");
    }
    out
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
        ability::{Direction, Partner},
        id::MarkId,
        time::Ticks,
    };
    use rstest::rstest;

    use super::*;
    use crate::test_support::{ability_specs, fire, lv, p, session};

    fn mark(id: &str, duration: Option<u64>) -> Effect {
        Effect::Mark {
            mark: MarkId::new(id),
            duration: duration.map(Ticks),
        }
    }

    #[rstest]
    fn a_collision_fires_the_medal_ability(mut session: GameSession) {
        let specs = ability_specs();
        let mut unit = MedalUnit::new(fire(), lv(1));
        let out =
            fire_abilities(&mut session, &specs, &mut unit, Happening::Collided(Partner::Enemy));
        assert_eq!(out.orders.len(), 1);
        assert_eq!(
            out.orders[0].actions,
            [EffectOrder::Apply(Effect::Push {
                dir: Direction::Outward,
                strength: p(2.0)
            })]
        );
        assert_eq!(out.orders[0].states, [mark("wet", Some(60))]);
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
