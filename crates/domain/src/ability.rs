//! メダルの能力（基本設計 3.14）。
//!
//! 能力は「いつ（[`Trigger`]） × 誰に（[`Target`]） × どこに（[`Area`]） ×
//! 何を（[`Effect`]）」の組で、 メダルごとの組み合わせは定義データに書く。
//! コードにあるのはこの語彙だけで、メダルIDによる分岐は書かない。 座標を使う判定（範囲内の対象・
//! 方向）は ECS が行い、ここは意味だけを決める。

use std::num::NonZeroU8;

use crate::{
    evolution::MedalLevel,
    id::{MarkId, MedalCode},
    motion::MotionEvent,
    positive::Positive,
    time::Ticks,
};

/// いつ発動するか。挙動ステータス（[`crate::motion`]）の出来事・衝突・時間・落下に結び付く。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 盤面にいる間ずっと（すり抜け・不動など、本体の性質）
    Always,
    /// 射出（スポーン）した瞬間
    Launch,
    /// 着地した（射出後と、浮かされた後の着地のたび）
    Land,
    /// 転がっていて止まった
    Rest,
    /// 止まっていたのが、押されて動き出した
    Wake,
    /// ぶつかった（同じ相手には1回）
    Collide(CollideWith),
    /// 射出からこの時間が経った（時限）
    Fuse(Ticks),
    /// 盤面にいる間、この間隔ごと
    Periodic(Ticks),
    /// 自分が奈落へ落ちた
    SelfFell,
    /// 盤面の他のメダルか敵が奈落へ落ちた
    OtherFell,
}

/// 衝突で発動する相手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollideWith {
    Medals,
    Enemies,
    Any,
}

/// ぶつかった相手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Partner {
    Medal,
    Enemy,
}

/// メダルに起きた出来事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Happening {
    /// 盤面に出た（射出・生成・初期配置）。常時の能力（[`Trigger::Always`]）が発動する
    Entered,
    Launched,
    Motion(MotionEvent),
    Collided(Partner),
    SelfFell,
    OtherFell,
    /// 時限・間隔の能力（番号）の時刻が来た。予定は [`crate::spec::abilities::AbilityBook::timers`]
    /// で引く
    Due(usize),
}

impl Trigger {
    /// この出来事で発動するか。
    pub fn fires_on(self, happening: Happening) -> bool {
        use Happening as H;
        matches!(
            (self, happening),
            (Self::Always, H::Entered)
                | (Self::Launch, H::Launched)
                | (Self::Land, H::Motion(MotionEvent::Landed))
                | (Self::Rest, H::Motion(MotionEvent::Rested))
                | (Self::Wake, H::Motion(MotionEvent::Woke))
                | (Self::SelfFell, H::SelfFell)
                | (Self::OtherFell, H::OtherFell)
                | (Self::Collide(CollideWith::Any), H::Collided(_))
                | (Self::Collide(CollideWith::Medals), H::Collided(Partner::Medal))
                | (Self::Collide(CollideWith::Enemies), H::Collided(Partner::Enemy))
        )
    }
}

/// 誰に効くか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Itself,
    Medals,
    Enemies,
    Both,
}

/// どこに効くか（発動したメダルの位置が基準）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Area {
    /// ぶつかった相手だけ（`Collide` 以外では発動したメダル自身）
    Touched,
    Radius(Positive),
    /// 射出方向の前方の帯
    Front {
        length: Positive,
        width: Positive,
    },
    /// 盤面全体
    Board,
}

/// 押す向き。前・後は射出方向（敵には進行方向）が基準。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Outward,
    Inward,
    Forward,
    Backward,
    Up,
    Swirl,
    TowardAbyss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnKind {
    /// 発動したメダルと同じ種・レベル
    Same,
    Normal,
    /// 素材メダルからランダム（セッションの乱数）
    RandomElement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnAt {
    Here,
    Catapult,
    /// 盤面の上空から降らせる
    Sky,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformInto {
    Normal,
    RandomElement,
}

/// 何をするか。
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Push {
        dir: Direction,
        strength: Positive,
    },
    /// 印を付ける。`duration` が `None` なら永続
    Mark {
        mark: MarkId,
        duration: Option<Ticks>,
    },
    Cleanse {
        harmful_only: bool,
    },
    /// 場: 発動した位置に `duration` の間残る範囲（炎の場・水たまり・毒沼）。中にいる対象（能力の
    /// `Target`）へ、 `linger` の長さの印を付け続ける。場を出ても `linger`
    /// の間は印が残る（[`Effect::zone_mark`]）
    Zone {
        mark: MarkId,
        radius: Positive,
        duration: Ticks,
        linger: Ticks,
    },
    /// メダルを生成する。`mark` は生成したメダルに付ける印（偽物・増殖の封印など）
    Spawn {
        kind: SpawnKind,
        count: NonZeroU8,
        at: SpawnAt,
        mark: Option<MarkId>,
        /// 生成したメダルに通常の進化報酬の資格を付ける（既定はなし。有価生成は回数・
        /// 報酬量と合わせてレビューする、基本設計 3.18）
        rewarded: bool,
    },
    Transform(TransformInto),
    /// 対象を消す（取り込む・食べる）。消えたメダルは落下と同じには数えない
    Remove,
    /// 対象のメダルを半分の大きさの2枚に割る
    Split,
    /// 進化ゲージにポイントを足す
    Gauge {
        points: u32,
    },
    /// 落ちた自分を盤面へ戻す（`uses` で回数を制限する）
    Revive {
        at: SpawnAt,
    },
    /// 盤面で直前に発動した能力を、
    /// この位置でもう一度発動する（[`crate::session::GameSession::fire_ability`]）
    Replay,
}

impl Effect {
    /// 対象の状態（印）だけを変える効果か。物理や盤面の変更を伴わない。
    /// メダルを浮かせる効果か（上へ押す）。浮いたメダルは飛行中に戻り、
    /// 着地でまた発動する（[`crate::motion::MedalMotion::lift`]）。
    pub fn lifts(&self) -> bool {
        matches!(self, Self::Push { dir: Direction::Up, .. })
    }

    pub fn is_state(&self) -> bool {
        matches!(self, Self::Mark { .. } | Self::Cleanse { .. })
    }

    /// 場が中にいる対象へ付ける印（長さは余韻 `linger`）。場でなければ `None`。
    pub fn zone_mark(&self) -> Option<Effect> {
        match self {
            Self::Zone { mark, linger, .. } => {
                Some(Self::Mark { mark: mark.clone(), duration: Some(*linger) })
            },
            _ => None,
        }
    }
}

/// 能力1つ。
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityDef {
    pub on: Trigger,
    pub target: Target,
    pub area: Area,
    pub effects: Vec<Effect>,
    /// 1個体あたりの発動回数の上限。`None` は無制限
    pub uses: Option<NonZeroU8>,
}

/// 発動した能力の参照（どのメダルの、どのレベルの、何番目の能力か）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityRef {
    pub medal: MedalCode,
    pub level: MedalLevel,
    pub index: usize,
}

/// 実行する能力。`replayed` は再生（[`Effect::Replay`]）で、もう一度発動する直前の能力。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FiredAbility<'a> {
    pub ability: &'a AbilityDef,
    pub replayed: Option<&'a AbilityDef>,
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("scaling an ability by {0} does not stay within range")]
pub struct ScaleError(pub f32);

impl AbilityDef {
    /// レベル倍率を掛けた能力。強さ・範囲・印と場の時間・ゲージのポイントに効き、
    /// 発動の時間（時限・間隔）と回数・個数には効かない。時間とポイントは四捨五入する。
    pub fn scaled(&self, factor: Positive) -> Result<Self, ScaleError> {
        let k = factor.get();
        let error = ScaleError(k);
        let mul = |v: Positive| v.checked_mul(factor).ok_or(error);
        // f64 で計算し、四捨五入した結果が整数型に収まるときだけ受け付ける
        let round = |v: f64, max: f64| {
            let r = (v * f64::from(k)).round();
            (r <= max).then_some(r).ok_or(error)
        };
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let ticks = |t: Ticks| round(t.0 as f64, u64::MAX as f64).map(|r| Ticks(r as u64));
        let area = match self.area {
            Area::Radius(r) => Area::Radius(mul(r)?),
            Area::Front { length, width } => {
                Area::Front { length: mul(length)?, width: mul(width)? }
            },
            other => other,
        };
        let effects = self
            .effects
            .iter()
            .map(|effect| {
                Ok(match effect {
                    Effect::Push { dir, strength } => {
                        Effect::Push { dir: *dir, strength: mul(*strength)? }
                    },
                    Effect::Mark { mark, duration } => Effect::Mark {
                        mark: mark.clone(),
                        duration: duration.map(ticks).transpose()?,
                    },
                    Effect::Zone { mark, radius, duration, linger } => Effect::Zone {
                        mark: mark.clone(),
                        radius: mul(*radius)?,
                        duration: ticks(*duration)?,
                        linger: ticks(*linger)?,
                    },
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    Effect::Gauge { points } => Effect::Gauge {
                        points: round(f64::from(*points), f64::from(u32::MAX))? as u32,
                    },
                    other => other.clone(),
                })
            })
            .collect::<Result<_, ScaleError>>()?;
        Ok(Self { area, effects, ..self.clone() })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::positive::fixture::p;

    fn wet() -> MarkId {
        MarkId::new("wet")
    }

    fn ability(area: Area, effects: Vec<Effect>) -> AbilityDef {
        AbilityDef {
            on: Trigger::Fuse(Ticks(60)),
            target: Target::Both,
            area,
            effects,
            uses: NonZeroU8::new(2),
        }
    }

    fn all_effects() -> Vec<Effect> {
        vec![
            Effect::Push { dir: Direction::Outward, strength: p(4.0) },
            Effect::Mark { mark: wet(), duration: Some(Ticks(10)) },
            Effect::Mark { mark: wet(), duration: None },
            Effect::Zone {
                mark: wet(),
                radius: p(2.0),
                duration: Ticks(100),
                linger: Ticks(20),
            },
            Effect::Gauge { points: 3 },
            Effect::Spawn {
                kind: SpawnKind::Same,
                count: NonZeroU8::MIN,
                at: SpawnAt::Here,
                mark: None,
                rewarded: false,
            },
            Effect::Remove,
        ]
    }

    #[rstest]
    fn scaling_by_one_changes_nothing() {
        let a = ability(Area::Front { length: p(5.0), width: p(1.0) }, all_effects());
        assert_eq!(a.scaled(p(1.0)), Ok(a));
    }

    #[rstest]
    fn scaling_grows_strength_area_durations_and_points() {
        let a = ability(Area::Radius(p(2.0)), all_effects());
        let scaled = a.scaled(p(1.25)).expect("in range");
        assert_eq!(scaled.area, Area::Radius(p(2.5)));
        assert_eq!(
            scaled.effects,
            [
                Effect::Push { dir: Direction::Outward, strength: p(5.0) },
                Effect::Mark { mark: wet(), duration: Some(Ticks(13)) },
                Effect::Mark { mark: wet(), duration: None },
                Effect::Zone {
                    mark: wet(),
                    radius: p(2.5),
                    duration: Ticks(125),
                    linger: Ticks(25)
                },
                Effect::Gauge { points: 4 },
                Effect::Spawn {
                    kind: SpawnKind::Same,
                    count: NonZeroU8::MIN,
                    at: SpawnAt::Here,
                    mark: None,
                    rewarded: false,
                },
                Effect::Remove,
            ]
        );
        assert_eq!(scaled.on, Trigger::Fuse(Ticks(60)), "trigger timing is not scaled");
        assert_eq!(scaled.uses, a.uses);
    }

    #[rstest]
    fn front_area_scales_both_sides() {
        let a = ability(Area::Front { length: p(4.0), width: p(1.0) }, vec![]);
        assert_eq!(
            a.scaled(p(2.0)).map(|a| a.area),
            Ok(Area::Front { length: p(8.0), width: p(2.0) })
        );
    }

    #[rstest]
    #[case::strength(ability(Area::Board, vec![Effect::Push { dir: Direction::Up, strength: p(f32::MAX) }]))]
    #[case::radius(ability(Area::Radius(p(f32::MAX)), vec![]))]
    #[case::duration(ability(Area::Board, vec![Effect::Mark { mark: wet(), duration: Some(Ticks(u64::MAX)) }]))]
    #[case::zone(ability(Area::Board, vec![Effect::Zone { mark: wet(), radius: p(1.0), duration: Ticks(u64::MAX), linger: Ticks(1) }]))]
    #[case::points(ability(Area::Board, vec![Effect::Gauge { points: u32::MAX }]))]
    fn scaling_out_of_range_is_an_error(#[case] a: AbilityDef) {
        assert_eq!(a.scaled(p(2.0)), Err(ScaleError(2.0)));
    }

    #[rstest]
    #[case::linger(ability(Area::Board, vec![Effect::Zone { mark: wet(), radius: p(1.0), duration: Ticks(1), linger: Ticks(u64::MAX) }]))]
    fn lingering_out_of_range_is_an_error(#[case] a: AbilityDef) {
        assert_eq!(a.scaled(p(2.0)), Err(ScaleError(2.0)));
    }

    #[rstest]
    fn a_zone_marks_those_inside_for_its_linger() {
        let zone = Effect::Zone {
            mark: wet(),
            radius: p(2.0),
            duration: Ticks(300),
            linger: Ticks(30),
        };
        assert_eq!(
            zone.zone_mark(),
            Some(Effect::Mark { mark: wet(), duration: Some(Ticks(30)) })
        );
        assert_eq!(Effect::Remove.zone_mark(), None);
    }

    use crate::motion::MotionEvent::{Landed, Rested, Woke};

    #[rstest]
    #[case::launch(Trigger::Launch, Happening::Launched, true)]
    #[case::launch_is_not_landing(Trigger::Launch, Happening::Motion(Landed), false)]
    #[case::land(Trigger::Land, Happening::Motion(Landed), true)]
    #[case::rest(Trigger::Rest, Happening::Motion(Rested), true)]
    #[case::rest_is_not_waking(Trigger::Rest, Happening::Motion(Woke), false)]
    #[case::wake(Trigger::Wake, Happening::Motion(Woke), true)]
    #[case::medal_hits_medal(
        Trigger::Collide(CollideWith::Medals),
        Happening::Collided(Partner::Medal),
        true
    )]
    #[case::medal_ignores_enemy(
        Trigger::Collide(CollideWith::Medals),
        Happening::Collided(Partner::Enemy),
        false
    )]
    #[case::enemy_hits_enemy(
        Trigger::Collide(CollideWith::Enemies),
        Happening::Collided(Partner::Enemy),
        true
    )]
    #[case::enemy_ignores_medal(
        Trigger::Collide(CollideWith::Enemies),
        Happening::Collided(Partner::Medal),
        false
    )]
    #[case::any_hits_medal(
        Trigger::Collide(CollideWith::Any),
        Happening::Collided(Partner::Medal),
        true
    )]
    #[case::any_hits_enemy(
        Trigger::Collide(CollideWith::Any),
        Happening::Collided(Partner::Enemy),
        true
    )]
    #[case::self_fell(Trigger::SelfFell, Happening::SelfFell, true)]
    #[case::other_fell(Trigger::OtherFell, Happening::OtherFell, true)]
    #[case::own_fall_is_not_other(Trigger::OtherFell, Happening::SelfFell, false)]
    #[case::always_on_entering(Trigger::Always, Happening::Entered, true)]
    #[case::launch_is_not_entering(Trigger::Launch, Happening::Entered, false)]
    #[case::due_is_matched_by_index_not_here(Trigger::Fuse(Ticks(1)), Happening::Due(0), false)]
    #[case::always_is_scheduled(Trigger::Always, Happening::Launched, false)]
    #[case::fuse_is_scheduled(Trigger::Fuse(Ticks(1)), Happening::Launched, false)]
    #[case::periodic_is_scheduled(Trigger::Periodic(Ticks(1)), Happening::Motion(Rested), false)]
    fn triggers_fire_on_their_happenings(
        #[case] trigger: Trigger,
        #[case] happening: Happening,
        #[case] fires: bool,
    ) {
        assert_eq!(trigger.fires_on(happening), fires);
    }

    #[rstest]
    #[case::up(Effect::Push { dir: Direction::Up, strength: p(1.0) }, true)]
    #[case::outward(Effect::Push { dir: Direction::Outward, strength: p(1.0) }, false)]
    #[case::remove(Effect::Remove, false)]
    fn only_upward_pushes_lift(#[case] effect: Effect, #[case] lifts: bool) {
        assert_eq!(effect.lifts(), lifts);
    }
    #[rstest]
    fn only_marks_and_cleansing_are_state_effects() {
        assert!(Effect::Mark { mark: wet(), duration: None }.is_state());
        assert!(Effect::Cleanse { harmful_only: true }.is_state());
        assert!(!Effect::Remove.is_state());
        assert!(
            !Effect::Zone {
                mark: wet(),
                radius: p(1.0),
                duration: Ticks(1),
                linger: Ticks(1)
            }
            .is_state()
        );
        assert!(!Effect::Replay.is_state());
    }
}
