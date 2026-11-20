//! インゲームHUDとリザルト画面。
//! 撃破数はドメインイベントから数える（Presentationだけの関心事のため）。

use bevy::{picking::events::PointerClick, prelude::*};
use bevy_intl::I18nText;
use domain::event::DomainEvent;

use super::{
    keys::{self, Arg, Ui},
    play_time::PlayTime,
    select::label,
};
use crate::{
    port::{DomainEventMsg, GameResult, HudModel, Request, SubmitRequest},
    state::AppState,
};

/// このセッションの撃破数。
#[derive(Resource, Debug, Default)]
struct Score {
    defeated: u32,
}

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct WaveText;

#[derive(Component)]
struct GrowthText;

#[derive(Component)]
struct GaugeFill;

type WaveLabel = (With<WaveText>, Without<HudText>, Without<GrowthText>);
type GrowthLabel = (With<GrowthText>, Without<HudText>, Without<WaveText>);

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Score>()
        .add_systems(OnEnter(AppState::InGame), spawn_hud)
        .add_systems(OnEnter(AppState::Result), spawn_result)
        .add_systems(Update, count_score)
        .add_systems(Update, refresh_hud.run_if(in_state(AppState::InGame)))
        .add_systems(Update, continue_with_keys.run_if(in_state(AppState::Result)));
}

fn count_score(mut events: MessageReader<DomainEventMsg>, mut score: ResMut<Score>) {
    for DomainEventMsg(event) in events.read() {
        match event {
            DomainEvent::GameStarted { .. } => *score = Score::default(),
            DomainEvent::EnemyDefeated { .. } => score.defeated += 1,
            _ => {},
        }
    }
}

fn hud_text(hud: &HudModel, score: &Score) -> I18nText {
    Ui::Hud.with([
        (Arg::Wave, hud.wave.to_string()),
        (Arg::Enemies, hud.alive_enemies.to_string()),
        (Arg::Defeated, score.defeated.to_string()),
        (Arg::Level, hud.level.to_string()),
        (Arg::Gauge, hud.evolution_percent.to_string()),
    ])
}

fn spawn_hud(
    mut commands: Commands,
    hud: Res<HudModel>,
    score: Res<Score>,
    server: Res<AssetServer>,
) {
    commands.spawn((
        DespawnOnExit(AppState::InGame),
        Node {
            position_type: PositionType::Absolute,
            left: px(24),
            top: px(24),
            width: px(224),
            padding: UiRect::all(px(20)),
            border: UiRect::all(px(1)),
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.018, 0.055, 0.08, 0.94)),
        BorderColor::all(Color::srgb(0.35, 0.31, 0.18)),
        children![
            label(Ui::Expedition.text(), 12.0, Color::srgb(0.42, 0.78, 0.75)),
            (
                WaveText,
                label(
                    Ui::HudWave.with([(Arg::Wave, hud.wave.to_string())]),
                    32.0,
                    Color::srgb(0.98, 0.86, 0.57)
                )
            ),
            (
                HudText,
                label(hud_text(&hud, &score), 19.0, Color::srgb(0.78, 0.87, 0.87))
            ),
            (
                Node { height: px(1), width: percent(100), ..default() },
                BackgroundColor(Color::srgb(0.12, 0.23, 0.25))
            ),
            (
                GrowthText,
                label(
                    Ui::HudGrowth.with([
                        (Arg::Level, hud.level.to_string()),
                        (Arg::Gauge, hud.evolution_percent.to_string())
                    ]),
                    15.0,
                    Color::srgb(0.5, 0.87, 0.8)
                )
            ),
            (
                Node {
                    width: percent(100),
                    height: px(7),
                    overflow: Overflow::clip(),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.04, 0.12, 0.15)),
                children![(
                    GaugeFill,
                    Node {
                        width: percent(hud.evolution_percent as f32),
                        height: percent(100),
                        border_radius: BorderRadius::all(px(4)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.19, 0.83, 0.7))
                )],
            ),
        ],
    ));
    commands.spawn((
        DespawnOnExit(AppState::InGame),
        Node {
            position_type: PositionType::Absolute,
            right: px(24),
            top: px(24),
            width: px(180),
            border_radius: BorderRadius::all(px(16)),
            padding: UiRect::all(px(16)),
            flex_direction: FlexDirection::Column,
            row_gap: px(12),
            ..default()
        },
        BackgroundColor(Color::srgba(0.018, 0.055, 0.08, 0.9)),
        children![
            (
                ImageNode::new(server.load(keys::PUSHER_PORTRAIT)),
                Node {
                    width: px(112),
                    height: px(92),
                    align_self: AlignSelf::Center,
                    ..default()
                }
            ),
            label(Ui::Controls.text(), 14.0, Color::srgb(0.57, 0.73, 0.75)),
        ],
    ));
}

/// 値が変わったときだけ引数を差し替える（bevy-intl が変更を検知して再描画する）。
fn refresh_hud(
    hud: Res<HudModel>,
    score: Res<Score>,
    mut texts: Query<&mut I18nText, With<HudText>>,
    mut waves: Query<&mut I18nText, WaveLabel>,
    mut growth: Query<&mut I18nText, GrowthLabel>,
    mut gauges: Query<&mut Node, With<GaugeFill>>,
) {
    if !hud.is_changed() && !score.is_changed() {
        return;
    }
    for mut text in &mut texts {
        *text = hud_text(&hud, &score);
    }
    for mut text in &mut waves {
        *text = Ui::HudWave.with([(Arg::Wave, hud.wave.to_string())]);
    }
    for mut text in &mut growth {
        *text = Ui::HudGrowth.with([
            (Arg::Level, hud.level.to_string()),
            (Arg::Gauge, hud.evolution_percent.to_string()),
        ]);
    }
    for mut gauge in &mut gauges {
        gauge.width = percent(hud.evolution_percent.min(100) as f32);
    }
}

fn spawn_result(mut commands: Commands, result: Option<Res<GameResult>>, score: Res<Score>) {
    // GameResult はリザルトへ移る前に必ず登録される。無い場合は今回の撃破数だけを出す
    let (best, new_record, play_time) = match result.as_deref() {
        Some(
            GameResult::Lost { best_defeated, new_record, play_time, .. }
            | GameResult::Won { best_defeated, new_record, play_time, .. },
        ) => (*best_defeated, *new_record, PlayTime(*play_time).to_string()),
        None => (score.defeated, false, String::new()),
    };
    let title = if matches!(result.as_deref(), Some(GameResult::Won { .. })) {
        Ui::Victory
    } else {
        Ui::GameOver
    };
    let record = if new_record {
        Ui::NewRecord
    } else {
        Ui::BestRecord
    };
    commands
        .spawn((
            DespawnOnExit(AppState::Result),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(12),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            children![
                label(title.text(), 64.0, Color::srgb(0.9, 0.65, 0.25)),
                label(
                    Ui::Defeated.with([(Arg::Defeated, score.defeated.to_string())]),
                    28.0,
                    Color::WHITE
                ),
                label(Ui::PlayTime.with([(Arg::Time, play_time)]), 24.0, Color::WHITE),
                label(
                    record.with([(Arg::Best, best.to_string())]),
                    24.0,
                    Color::srgb(0.95, 0.8, 0.3)
                ),
                label(Ui::Continue.text(), 20.0, Color::srgb(0.7, 0.7, 0.7)),
            ],
        ))
        .observe(|_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
            submit.write(SubmitRequest(Request::Menu));
        });
}

fn continue_with_keys(keys: Res<ButtonInput<KeyCode>>, mut submit: MessageWriter<SubmitRequest>) {
    if keys.any_just_pressed([KeyCode::Enter, KeyCode::Space]) {
        submit.write(SubmitRequest(Request::Menu));
    }
}
