//! インゲームHUDとリザルト画面。
//! 撃破数はドメインイベントから数える（Presentationだけの関心事のため）。

use bevy::{picking::events::PointerClick, prelude::*};
use domain::event::DomainEvent;

use super::select::text;
use crate::{
    port::{DomainEventMsg, GameResult, HudModel, MenuRequested},
    state::AppState,
};

/// このセッションの撃破数。
#[derive(Resource, Debug, Default)]
struct Score {
    defeated: u32,
}

#[derive(Component)]
struct HudText;

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

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        DespawnOnExit(AppState::InGame),
        Node { position_type: PositionType::Absolute, left: px(24), top: px(20), ..default() },
        children![(HudText, text("", 22.0, Color::srgb(0.9, 0.88, 0.75)))],
    ));
    commands.spawn((
        DespawnOnExit(AppState::InGame),
        Node { position_type: PositionType::Absolute, right: px(24), bottom: px(20), ..default() },
        children![text(
            "A/D or ←/→: aim\nSpace / click / tap: launch",
            16.0,
            Color::srgb(0.55, 0.6, 0.55)
        )],
    ));
}

fn refresh_hud(hud: Res<HudModel>, score: Res<Score>, mut texts: Query<&mut Text, With<HudText>>) {
    if !hud.is_changed() && !score.is_changed() {
        return;
    }
    for mut text in &mut texts {
        text.0 = format!(
            "WAVE {}\nENEMIES {}\nDEFEATED {}\nLV {}",
            hud.wave, hud.alive_enemies, score.defeated, hud.level
        );
    }
}

fn spawn_result(mut commands: Commands, result: Option<Res<GameResult>>, score: Res<Score>) {
    let headline = match result.as_deref() {
        Some(GameResult::Lost { .. }) | None => "GAME OVER",
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
                text(headline, 64.0, Color::srgb(0.9, 0.3, 0.25)),
                text(format!("Defeated {}", score.defeated), 28.0, Color::WHITE),
                text("Click or press Enter", 20.0, Color::srgb(0.7, 0.7, 0.7)),
            ],
        ))
        .observe(|_: On<PointerClick>, mut menu: MessageWriter<MenuRequested>| {
            menu.write(MenuRequested);
        });
}

fn continue_with_keys(keys: Res<ButtonInput<KeyCode>>, mut menu: MessageWriter<MenuRequested>) {
    if keys.any_just_pressed([KeyCode::Enter, KeyCode::Space]) {
        menu.write(MenuRequested);
    }
}
