//! Presentation層: 2.5Dの見た目・入力・UI・演出（基本設計 5章, 11章）。
//! Infrastructureを参照しない。`port` の Message・Resource・Component だけを使う。

mod camera;
mod feedback;
mod hud;
mod input;
mod select;
mod visuals;

use bevy::prelude::*;

pub(crate) fn plugin(app: &mut App) {
    app.add_plugins((
        camera::plugin,
        visuals::plugin,
        input::plugin,
        select::plugin,
        hud::plugin,
        feedback::plugin,
    ));
}
