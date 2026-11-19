//! Presentation層: 2.5Dの見た目・入力・UI・演出（基本設計 5章, 11章）。
//! Infrastructureを参照しない。`port` の Message・Resource・Component だけを使う。

mod camera;
mod evolution;
mod feedback;
mod hud;
mod input;
pub mod keys;
pub mod outgame;
pub mod play_time;
mod select;
mod visuals;

use bevy::prelude::*;
use bevy_intl::{I18nConfig, I18nPlugin};

pub(crate) fn plugin(app: &mut App) {
    // 表示文はロケールのカタログ（game/locales）にあり、bundle-only でバイナリへ埋め込まれる
    app.add_plugins(I18nPlugin::with_config(I18nConfig {
        default_lang: keys::DEFAULT_LANG.to_owned(),
        fallback_lang: keys::FALLBACK_LANG.to_owned(),
        ..default()
    }));
    app.add_plugins((
        camera::plugin,
        visuals::plugin,
        input::plugin,
        select::plugin,
        hud::plugin,
        evolution::plugin,
        feedback::plugin,
        outgame::plugin,
    ));
}
