//! キャラクター選択画面（要件3.1）。ボタン（Observer）と数字キーで選ぶ。

use bevy::{picking::events::PointerClick, prelude::*};
use bevy_intl::I18nText;

use super::keys::{Arg, Ui};
use crate::{
    port::{CharacterRoster, StartRequested},
    state::AppState,
};

const NUMBER_KEYS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

pub(super) fn plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::CharacterSelect), spawn_menu)
        .add_systems(Update, select_with_keys.run_if(in_state(AppState::CharacterSelect)));
}

/// 翻訳される表示文。文言はロケールのカタログにある。
pub(super) fn label(text: I18nText, size: f32, color: Color) -> impl Bundle {
    (
        text,
        TextFont { font_size: FontSize::Px(size), ..default() },
        TextColor(color),
    )
}

fn spawn_menu(mut commands: Commands, roster: Res<CharacterRoster>) {
    let root = commands
        .spawn((
            DespawnOnExit(AppState::CharacterSelect),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(16),
                ..default()
            },
            children![
                label(Ui::Title.text(), 72.0, Color::srgb(0.95, 0.8, 0.3)),
                label(Ui::ChoosePusher.text(), 24.0, Color::srgb(0.7, 0.75, 0.7)),
            ],
        ))
        .id();

    for (index, entry) in roster.0.iter().enumerate() {
        let character = entry.id.clone();
        let caption = Ui::CharacterEntry
            .with([(Arg::Index, (index + 1).to_string()), (Arg::Name, entry.name.clone())]);
        let button = commands
            .spawn((
                Node {
                    width: px(320),
                    padding: UiRect::all(px(14)),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.16, 0.2, 0.14)),
                children![label(caption, 28.0, Color::srgb(0.92, 0.92, 0.85))],
            ))
            .observe(move |_: On<PointerClick>, mut start: MessageWriter<StartRequested>| {
                start.write(StartRequested { character: character.clone() });
            })
            .id();
        commands.entity(root).add_child(button);
    }
}

fn select_with_keys(
    keys: Res<ButtonInput<KeyCode>>,
    roster: Res<CharacterRoster>,
    mut start: MessageWriter<StartRequested>,
) {
    let picked = NUMBER_KEYS.iter().position(|&k| keys.just_pressed(k));
    if let Some(entry) = picked.and_then(|i| roster.0.get(i)) {
        start.write(StartRequested { character: entry.id.clone() });
    }
}
