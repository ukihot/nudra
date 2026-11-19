//! 入力 → 要求Message（基本設計 11章）。キーボード・マウス・タッチを同じ操作に対応付ける。
//! 状態の変更はしない。照準の表示は読み取りモデル `AimModel` から描く。

use bevy::prelude::*;

use crate::{
    port::{AimModel, AimRequested, LaunchRequested},
    state::AppState,
};

/// キー押下中の旋回速度（rad/s）
const AIM_SPEED: f32 = 1.4;
/// タッチのドラッグ量（px）あたりの旋回量（rad）
const TOUCH_AIM_PER_PX: f32 = 0.008;
const AIM_GUIDE_LENGTH: f32 = 3.5;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (read_controls, draw_aim_guide).run_if(in_state(AppState::InGame)),
    );
    #[cfg(debug_assertions)]
    app.add_systems(Update, debug_level_up.run_if(in_state(AppState::InGame)));
}

fn read_controls(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    time: Res<Time>,
    mut aim: MessageWriter<AimRequested>,
    mut launch: MessageWriter<LaunchRequested>,
) {
    let pressed = |a, b| keys.pressed(a) || keys.pressed(b);
    // +yaw は左回り（画面左）
    let steer = f32::from(u8::from(pressed(KeyCode::ArrowLeft, KeyCode::KeyA)))
        - f32::from(u8::from(pressed(KeyCode::ArrowRight, KeyCode::KeyD)));
    let drag: f32 = touches.iter().map(|t| t.delta().x).sum();
    let delta = steer * AIM_SPEED * time.delta_secs() - drag * TOUCH_AIM_PER_PX;
    if delta != 0.0 {
        aim.write(AimRequested { delta });
    }

    // 押している間は要求し続ける。実際の連射間隔はDomainのクールダウンが決める
    let firing = keys.pressed(KeyCode::Space)
        || mouse.pressed(MouseButton::Left)
        || touches.iter().next().is_some();
    if firing {
        launch.write(LaunchRequested);
    }
}

fn draw_aim_guide(aim: Res<AimModel>, mut gizmos: Gizmos) {
    if aim.direction != Vec3::ZERO {
        let tip = aim.origin + aim.direction * AIM_GUIDE_LENGTH;
        gizmos.arrow(aim.origin, tip, Color::srgb(1.0, 0.85, 0.3));
    }
}

/// 【仮】成長の選択方式は未確定のため、開発ビルドだけ数字キーで強化を試せるようにする。
#[cfg(debug_assertions)]
fn debug_level_up(
    keys: Res<ButtonInput<KeyCode>>,
    mut requests: MessageWriter<crate::port::LevelUpRequested>,
) {
    use domain::{positive::Positive, session::Upgrade, spec::MedalType};
    let factor = |v| Positive::new(v).expect("constant is positive");
    let upgrade = if keys.just_pressed(KeyCode::Digit1) {
        Upgrade::MedalMass { medal: MedalType::Normal, factor: factor(1.5) }
    } else if keys.just_pressed(KeyCode::Digit2) {
        Upgrade::PusherSpeed { factor: factor(1.25) }
    } else if keys.just_pressed(KeyCode::Digit3) {
        Upgrade::PusherForce { factor: factor(1.5) }
    } else {
        return;
    };
    requests.write(crate::port::LevelUpRequested(upgrade));
}
