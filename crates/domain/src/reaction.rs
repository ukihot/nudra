//! 印どうしの反応と、効果が対象に当たったときの処理（基本設計 3.14）。
//!
//! 反応は「対象にすでにある印 `on` に、別の印 `by` が付けられたら、効果を足す」という定義データ。
//! - 反応するのは、当たる前から対象にあった印だけ（同じ能力が一度に付ける印どうしでは反応しない）
//! - 反応は1段まで。反応で付いた印から、さらに反応は起こさない（組み合わせの爆発と無限連鎖を防ぐ）

use crate::{
    ability::Effect,
    id::MarkId,
    mark::{Conditions, MarkDef},
    time::Tick,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ReactionDef {
    pub on: MarkId,
    pub by: MarkId,
    pub effects: Vec<Effect>,
    /// 反応したら `on` の印を外す（濡れが蒸発する など）
    pub consume: bool,
}

/// 当たった結果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hit {
    /// 対象の印が変わった（効き目を反映し直す）
    pub changed: bool,
    /// 反応で増えた、状態以外の効果（押す・消す など。ECS が対象の位置で実行する）
    pub effects: Vec<Effect>,
}

/// `incoming` のうち状態の効果（印・浄化）を対象に適用し、反応を解決する。
/// 状態以外の効果は呼び出し側が実行するため、ここでは扱わない。
pub fn hit<'a>(
    target: &mut Conditions,
    incoming: &[Effect],
    now: Tick,
    marks: impl Fn(&MarkId) -> Option<&'a MarkDef> + Copy,
    reactions: &[ReactionDef],
) -> Hit {
    // 反応は当たる前の状態で決める（同じ当たりで付く印どうしでは反応しない）
    let fired: Vec<&ReactionDef> = reactions
        .iter()
        .filter(|r| {
            target.has(&r.on)
                && incoming
                    .iter()
                    .any(|e| matches!(e, Effect::Mark { mark, .. } if *mark == r.by))
        })
        .collect();
    let mut out = Hit::default();
    for effect in incoming {
        out.changed |= apply_state(target, effect, now, marks);
    }
    // 反応の効果は解決済みとして適用し、ここから先の反応は調べない（1段まで）
    for reaction in fired {
        if reaction.consume {
            out.changed |= target.remove(&reaction.on);
        }
        for effect in &reaction.effects {
            if effect.is_state() {
                out.changed |= apply_state(target, effect, now, marks);
            } else {
                out.effects.push(effect.clone());
            }
        }
    }
    out
}

/// 状態の効果を1つ適用する。変わったら `true`。未定義の印と状態以外の効果は何もしない。
fn apply_state<'a>(
    target: &mut Conditions,
    effect: &Effect,
    now: Tick,
    marks: impl Fn(&MarkId) -> Option<&'a MarkDef>,
) -> bool {
    match effect {
        Effect::Mark { mark, duration } => marks(mark).is_some_and(|def| {
            target.apply(def, duration.map(|d| now + d));
            true
        }),
        Effect::Cleanse { harmful_only } => target.cleanse(*harmful_only, marks),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        ability::Direction,
        mark::fixture::{book, grow, poison, sand, wet},
        positive::fixture::p,
        time::Ticks,
    };

    fn mark(def: &MarkDef, duration: Option<u64>) -> Effect {
        Effect::Mark {
            mark: def.id().clone(),
            duration: duration.map(Ticks),
        }
    }

    fn push() -> Effect {
        Effect::Push { dir: Direction::Outward, strength: p(3.0) }
    }

    /// 濡れ + 毒 → 弾けて砂が残る（濡れは消える）。砂 + 毒 →
    /// 消える（2段目なので起きてはならない）。
    #[fixture]
    fn reactions() -> Vec<ReactionDef> {
        vec![
            ReactionDef {
                on: wet().id().clone(),
                by: poison().id().clone(),
                effects: vec![push(), mark(&sand(), Some(30))],
                consume: true,
            },
            ReactionDef {
                on: sand().id().clone(),
                by: poison().id().clone(),
                effects: vec![Effect::Remove],
                consume: false,
            },
        ]
    }

    fn ids(c: &Conditions) -> Vec<&str> {
        c.marks().iter().map(|m| m.mark.as_str()).collect()
    }

    #[rstest]
    fn marks_are_applied_with_deadlines_from_now(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        let out = hit(
            &mut target,
            &[mark(&wet(), Some(10)), mark(&grow(), None)],
            Tick(5),
            book,
            &reactions,
        );
        assert_eq!(out, Hit { changed: true, effects: vec![] });
        assert_eq!(target.marks()[0].until, Some(Tick(15)));
        assert_eq!(target.marks()[1].until, None);
    }

    #[rstest]
    fn a_mark_on_a_marked_target_reacts(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        target.apply(&wet(), None);

        let out = hit(&mut target, &[mark(&poison(), Some(60))], Tick(100), book, &reactions);

        assert_eq!(out.effects, [push()], "only non-state effects are returned");
        assert_eq!(
            ids(&target),
            ["poison", "sand"],
            "wet is consumed, sand comes from the reaction"
        );
        assert_eq!(target.marks()[1].until, Some(Tick(130)));
    }

    #[rstest]
    fn reactions_go_only_one_step_deep(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        target.apply(&wet(), None);
        let out = hit(&mut target, &[mark(&poison(), None)], Tick(0), book, &reactions);
        assert!(
            !out.effects.contains(&Effect::Remove),
            "sand from a reaction must not react"
        );
    }

    #[rstest]
    fn marks_of_the_same_hit_do_not_react_with_each_other(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        let out = hit(
            &mut target,
            &[mark(&wet(), None), mark(&poison(), None)],
            Tick(0),
            book,
            &reactions,
        );
        assert!(out.effects.is_empty());
        assert_eq!(ids(&target), ["wet", "poison"]);
    }

    #[rstest]
    fn a_target_without_the_mark_does_not_react(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        target.apply(&grow(), None);
        let out = hit(&mut target, &[mark(&poison(), None)], Tick(0), book, &reactions);
        assert!(out.effects.is_empty());
        assert_eq!(ids(&target), ["grow", "poison"]);
    }

    #[rstest]
    fn cleansing_is_applied_in_order(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        target.apply(&sand(), None);
        let out = hit(
            &mut target,
            &[Effect::Cleanse { harmful_only: true }, mark(&wet(), None)],
            Tick(0),
            book,
            &reactions,
        );
        assert!(out.changed);
        assert_eq!(ids(&target), ["wet"]);
    }

    #[rstest]
    fn non_state_effects_are_left_to_the_caller(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        let out = hit(&mut target, &[push(), Effect::Remove], Tick(0), book, &reactions);
        assert_eq!(out, Hit::default());
        assert!(target.is_empty());
    }

    #[rstest]
    fn unknown_marks_are_ignored(reactions: Vec<ReactionDef>) {
        let mut target = Conditions::default();
        let unknown = Effect::Mark { mark: MarkId::new("unknown"), duration: None };
        assert_eq!(
            hit(&mut target, &[unknown], Tick(0), book, &reactions),
            Hit::default()
        );
    }
}
