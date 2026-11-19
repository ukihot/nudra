//! 進化画面と、所持メダルの選択。表示内容は読み取りモデル、操作は指図で渡す。

use bevy::{
    picking::{events::PointerClick, hover::Hovered},
    prelude::*,
};
use domain::evolution::Evolution;

use super::{
    keys::{self, Ui},
    select::label,
};
use crate::{
    port::{EvolutionModel, MedalModel, Request, SubmitRequest},
    state::{AppState, GamePhase},
};

#[derive(Component)]
struct MedalBar;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(GamePhase::Evolving), spawn_evolution)
        .add_systems(Update, refresh_medals.run_if(in_state(AppState::InGame)))
        .add_systems(Update, select_next.run_if(in_state(GamePhase::Running)));
}

fn spawn_evolution(mut commands: Commands, model: Res<EvolutionModel>) {
    let root = commands
        .spawn((
            DespawnOnExit(GamePhase::Evolving),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(10),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            GlobalZIndex(10),
            BackgroundColor(Color::srgba(0.04, 0.06, 0.04, 0.96)),
            children![label(Ui::Evolve.text(), 36.0, Color::WHITE)],
        ))
        .id();
    for choice in &model.0 {
        let caption = match choice.evolution {
            Evolution::LevelUp(_) => Ui::LevelUp,
            Evolution::Fuse(..) => Ui::Fuse,
            Evolution::Acquire(_) => Ui::NewMedal,
        };
        let color = if choice.enabled {
            Color::srgb(0.3, 0.4, 0.2)
        } else {
            Color::srgb(0.13, 0.13, 0.13)
        };
        let action = choice.evolution.clone();
        let enabled = choice.enabled;
        let button = commands
            .spawn((
                Hovered::default(),
                Node {
                    width: px(320),
                    padding: UiRect::all(px(10)),
                    column_gap: px(16),
                    ..default()
                },
                BackgroundColor(color),
                children![
                    label(caption.text(), 22.0, Color::WHITE),
                    label(keys::medal_name(&choice.result), 22.0, Color::WHITE)
                ],
            ))
            .observe(move |_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
                if enabled {
                    submit.write(SubmitRequest(Request::Evolve(action.clone())));
                }
            })
            .id();
        commands.entity(root).add_child(button);
    }
}

fn refresh_medals(
    mut commands: Commands,
    medals: Res<MedalModel>,
    bars: Query<Entity, With<MedalBar>>,
) {
    if !medals.is_changed() {
        return;
    }
    for bar in &bars {
        commands.entity(bar).despawn();
    }
    let root = commands
        .spawn((
            MedalBar,
            DespawnOnExit(AppState::InGame),
            Node {
                position_type: PositionType::Absolute,
                left: px(24),
                bottom: px(24),
                column_gap: px(8),
                ..default()
            },
        ))
        .id();
    for medal in &medals.owned {
        let selected = medals.selected.as_ref() == Some(medal);
        let medal = medal.clone();
        let button = commands
            .spawn((
                Hovered::default(),
                Node { padding: UiRect::all(px(10)), ..default() },
                BackgroundColor(if selected {
                    Color::srgb(0.35, 0.4, 0.15)
                } else {
                    Color::srgb(0.12, 0.15, 0.1)
                }),
                children![label(keys::medal_name(&medal), 20.0, Color::WHITE)],
            ))
            .observe(move |_: On<PointerClick>, mut submit: MessageWriter<SubmitRequest>| {
                submit.write(SubmitRequest(Request::SelectMedal(medal.clone())));
            })
            .id();
        commands.entity(root).add_child(button);
    }
}

fn select_next(
    keys: Res<ButtonInput<KeyCode>>,
    medals: Res<MedalModel>,
    mut submit: MessageWriter<SubmitRequest>,
) {
    if keys.just_pressed(KeyCode::Tab) && !medals.owned.is_empty() {
        let current = medals
            .owned
            .iter()
            .position(|m| Some(m) == medals.selected.as_ref())
            .unwrap_or(0);
        submit.write(SubmitRequest(Request::SelectMedal(
            medals.owned[(current + 1) % medals.owned.len()].clone(),
        )));
    }
}
